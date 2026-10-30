use clap::{Parser, Subcommand};
use unipi_host::{
    data_dir::HostPaths, devices::DeviceStore, endpoint::ALPN, pairing::now_unix, protocol::Handler,
};
#[derive(Parser)]
#[command(
    name = "unipi-host",
    version,
    about = "UniPi host: one per user — agent sessions, terminal, files, git, browser and cron for the UniPi app (this PC's app over app.sock, paired phones over iroh)"
)]
struct Cli {
    #[arg(long, global = true)]
    data_dir: Option<std::path::PathBuf>,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    Serve {
        #[arg(long, default_value = "n0")]
        relay: String,
        #[arg(long)]
        relay_only: bool,
        /// Who runs this host (shown by `status` and the desktop app):
        /// app (started by the app's supervisor) or manual (a terminal).
        /// v2 has no service install, so there is no `service` mode.
        #[arg(long, value_parser = ["app", "manual"])]
        mode: Option<String>,
        /// Exit when this process exits (the desktop app's sidecar).
        #[arg(long, value_name = "PID")]
        exit_with_parent: Option<u32>,
        /// Drop this extension id (repeatable; same as `oino --exclude-extension`).
        #[arg(long = "exclude-extension", value_name = "ID")]
        exclude_extension: Vec<String>,
        /// Start without the UniPi extensions (providers stay). Safe mode also does this by
        /// itself after two failed starts (spec-host §4.4).
        #[arg(long = "no-extensions")]
        no_extensions: bool,
    },
    /// Is a host running on this data dir, and who runs it.
    /// Exit 0 = running, 3 = not running.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Show a pairing QR + link for a phone (needs `unipi-host serve` running).
    Pair {
        /// How long the code works (e.g. 10m, 90s; at most 60m).
        #[arg(long, default_value = "10m")]
        ttl: String,
        /// JSON lines for the UniPi desktop app: the link first, then one line
        /// per state change (paired / expired). Closing stdin cancels.
        #[arg(long)]
        json: bool,
    },
    Devices,
    Revoke {
        target: String,
    },
    /// Port forwarding to the phone's Preview page (off by default).
    Forward {
        #[command(subcommand)]
        action: ForwardAction,
    },
    /// The host's own browser (off by default): CloakBrowser (a stealth
    /// Chromium, downloaded on first use) or Google Chrome, with its own
    /// profiles, driven over CDP by the phone's Preview page and by agents.
    Browser {
        #[command(subcommand)]
        action: BrowserAction,
    },
    /// Which folders the app's Files screen and file links may open besides
    /// $HOME and live session folders. Defaults (only those that exist):
    /// /tmp, /var/tmp, /mnt, /media, /run/media/$USER, /srv, /opt.
    /// System folders (/, /etc, /usr, /proc, /sys, /dev, /boot, /root, the
    /// rest of /var and /run) and other users' homes are always refused.
    Fs {
        #[command(subcommand)]
        action: FsAction,
    },
    /// Requests from phones to switch a feature on ("Ask the PC to turn this
    /// on"). Nothing is ever switched on by itself: approve or deny here or in
    /// the UniPi desktop app on this PC.
    Requests {
        #[command(subcommand)]
        action: Option<RequestsAction>,
    },
    /// What phones may use on this PC (forwarding, browser, desktop, push) and
    /// the pending requests. `--json` is what the UniPi desktop app reads.
    Sharing {
        #[command(subcommand)]
        action: SharingAction,
    },
    /// Update unipi-host from the release manifest (sha256-checked, atomic
    /// replace). v2 has no service install: after the replace the host exits —
    /// the app restarts the host it spawned itself, a manual `serve` is rerun
    /// (`specs/host.md` §9.7).
    Update {
        /// Only say whether a newer version exists (exit 0 either way).
        #[arg(long, conflicts_with = "mode")]
        check: bool,
        /// Replace this file instead of the running executable (tests).
        #[arg(long, hide = true)]
        exe: Option<std::path::PathBuf>,
        /// Owner setting: when `serve` finds a newer host, `prompt` (raise a
        /// notice for the app; the default) or `auto` (install it and exit).
        #[arg(long, value_parser = ["prompt", "auto"])]
        mode: Option<String>,
        /// Print the result as JSON (for the desktop app).
        #[arg(long)]
        json: bool,
    },
    /// Send a file to the phone: an APK to install, or any file to save/open.
    /// Needs `unipi-host serve` running and push switched on
    /// (`unipi-host push enable`, off by default).
    #[command(args_conflicts_with_subcommands = true, subcommand_negates_reqs = true)]
    Push {
        #[command(subcommand)]
        action: Option<PushAction>,
        /// The file to send.
        #[arg(required = true)]
        file: Option<std::path::PathBuf>,
        /// Only these paired devices (id, id prefix ≥ 8 chars, or name). Default: every allowed one.
        #[arg(long)]
        device: Vec<String>,
        /// A short note shown on the phone.
        #[arg(long)]
        note: Option<String>,
        /// Name shown on the phone (default: the file name).
        #[arg(long)]
        name: Option<String>,
        /// Return as soon as the push is queued instead of waiting for the phone.
        #[arg(long)]
        no_wait: bool,
    },
    /// Scheduled `pi -p` jobs, run by `unipi-host serve` itself (no pi
    /// window needed). See `docs/m7/CRON.md`.
    Cron {
        #[command(subcommand)]
        action: CronAction,
    },
}
#[derive(Subcommand)]
enum RequestsAction {
    /// List requests (pending first). The default.
    List {
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
    /// Switch the feature on for the asking device (only that device).
    Approve {
        id: String,
        /// Switch it back off after this long (e.g. 1h, 30m, 2d). Default: until you turn it off.
        #[arg(long = "for")]
        for_: Option<String>,
    },
    /// Say no (nothing changes on this PC).
    Deny { id: String },
}
#[derive(Subcommand)]
enum SharingAction {
    /// Every switch, the paired devices and the requests.
    Status {
        #[arg(long)]
        json: bool,
    },
}
#[derive(Subcommand)]
enum FsAction {
    /// Let the app open (and write in) this folder and everything under it.
    /// Also lifts an earlier `fs deny` on it.
    Allow { path: std::path::PathBuf },
    /// Stop the app opening this folder (an allowed one, a default root like
    /// /tmp, or a subfolder of one). $HOME can't be denied.
    Deny { path: std::path::PathBuf },
    /// Show the folders the app can open now, and the saved allow/deny lists.
    List,
}
#[derive(Subcommand)]
enum PushAction {
    /// Allow pushes (to every paired device, or only `--device …`).
    Enable {
        /// Only these paired devices (id, id prefix ≥ 8 chars, or name). Replaces the list.
        #[arg(long)]
        device: Vec<String>,
    },
    /// Stop allowing pushes (pending ones stay until they expire or are cancelled).
    Disable,
    /// Show the push settings and what is pending.
    Status,
    /// Cancel a pending push (or all with no id).
    Cancel { id: Option<String> },
}
#[derive(Subcommand)]
enum CronAction {
    /// List every job.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Show one job and its run history.
    Show {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Add a job. `--schedule`/`--at` take plain language ("every 5
    /// minutes", "every day at 9:00", "weekdays at 18:30", "every monday
    /// 08:00", "hourly", "daily", "in 2 hours", "tomorrow at 9") as well
    /// as a 6-field cron (seconds first, e.g. "0 30 9 * * MON-FRI") or
    /// `at:<RFC3339>` for a one-time run.
    Add {
        /// A short slug (default: made from --name).
        #[arg(long)]
        id: Option<String>,
        #[arg(long)]
        name: String,
        #[arg(long)]
        prompt: String,
        /// A recurring schedule (plain language or 6-field cron). Omit
        /// when using `--at` for a one-time run.
        #[arg(long, conflicts_with = "at")]
        schedule: Option<String>,
        /// A one-time run ("in 2 hours", "tomorrow at 9", or an exact
        /// RFC3339 instant). Omit when using `--schedule`.
        #[arg(long, conflicts_with = "schedule")]
        at: Option<String>,
        /// IANA zone name (default: this PC's).
        #[arg(long)]
        tz: Option<String>,
        /// The job's working directory (must exist; default: here).
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        thinking: Option<String>,
        #[arg(long)]
        disabled: bool,
        #[arg(long)]
        time_limit_s: Option<u64>,
        #[arg(long)]
        cost_cap: Option<f64>,
        /// Notify channels on this job's events (comma-separated:
        /// ntfy,gotify,telegram). Default: the user's own enabled channels.
        #[arg(long, value_delimiter = ',')]
        notify: Vec<String>,
        /// Run statuses that trigger a notification (comma-separated:
        /// done,failed,skipped). Default: failed.
        #[arg(long, value_delimiter = ',')]
        notify_on: Vec<String>,
        /// Wake this pi session (a session file path, or a session id)
        /// once the run ends.
        #[arg(long, conflicts_with = "wake_here")]
        wake_session: Option<String>,
        /// Wake *this* session (reads `$PI_SESSION_FILE`, set by pi's bash
        /// tool) once the run ends.
        #[arg(long, conflicts_with = "wake_session")]
        wake_here: bool,
        #[arg(long)]
        json: bool,
    },
    /// Edit a job's fields (same flags as `add`; only what's given changes).
    Edit {
        id: String,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        prompt: Option<String>,
        #[arg(long, conflicts_with = "at")]
        schedule: Option<String>,
        #[arg(long, conflicts_with = "schedule")]
        at: Option<String>,
        #[arg(long)]
        tz: Option<String>,
        #[arg(long)]
        cwd: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        thinking: Option<String>,
        #[arg(long)]
        time_limit_s: Option<u64>,
        #[arg(long)]
        cost_cap: Option<f64>,
        /// Replaces the notify channels (comma-separated; "none" clears
        /// it back to the user's default).
        #[arg(long, value_delimiter = ',')]
        notify: Vec<String>,
        /// Replaces the notify events (comma-separated; "none" clears it
        /// back to the default, failed-only).
        #[arg(long, value_delimiter = ',')]
        notify_on: Vec<String>,
        /// Replaces the linked session ("none" removes it).
        #[arg(long, conflicts_with = "wake_here")]
        wake_session: Option<String>,
        /// Links *this* session (reads `$PI_SESSION_FILE`).
        #[arg(long, conflicts_with = "wake_session")]
        wake_here: bool,
        #[arg(long)]
        json: bool,
    },
    /// Remove a job (and its run history).
    Rm {
        id: String,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    /// Run a job now, bypassing its schedule (still subject to overlap/concurrency).
    Run {
        id: String,
        #[arg(long)]
        json: bool,
    },
    /// Kill a job's running process.
    Stop {
        id: String,
    },
    /// Run history, newest first.
    Runs {
        /// Only this job's runs (default: every job's).
        id: Option<String>,
        #[arg(long)]
        limit: Option<usize>,
        #[arg(long)]
        json: bool,
    },
    /// A run's captured output (stdout+stderr).
    Log {
        run_id: String,
        #[arg(long)]
        json: bool,
    },
    /// Stop starting new runs (running ones finish).
    Pause,
    Resume,
    /// Next fire per job, what's running, paused or not.
    Status {
        #[arg(long)]
        json: bool,
    },
    /// Allow phones to use cron ops (to every paired device, or only
    /// `--device …`); this PC's own app always has them.
    Share {
        /// Only these paired devices (id, id prefix ≥ 8 chars, or name). Replaces the list.
        #[arg(long)]
        device: Vec<String>,
    },
    /// Stop allowing phones to use cron ops.
    Unshare,
    /// Show or change the scheduler settings (`concurrency`, `missed_window_s`,
    /// `retention_runs`, `retention_days`, `default_time_limit_s`, `paused`,
    /// `notify_default_events`; `<agent dir>/cron/settings.json`). With no flags,
    /// just shows the settings. The ntfy topic is a notify-extension setting.
    Settings {
        #[arg(long)]
        concurrency: Option<u32>,
        #[arg(long)]
        missed_window_s: Option<u64>,
        #[arg(long)]
        retention_runs: Option<u32>,
        #[arg(long)]
        retention_days: Option<u32>,
        #[arg(long)]
        default_time_limit_s: Option<u64>,
        #[arg(long)]
        paused: Option<bool>,
        /// Comma-separated run outcomes (e.g. `failed,timeout`).
        #[arg(long)]
        notify_default_events: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Sends one real test notification on this job's notify channels.
    TestNotify {
        id: String,
        #[arg(long)]
        json: bool,
    },
}
#[derive(Subcommand)]
enum BrowserAction {
    /// Turn the remote browser on (no restart of `serve` needed).
    Enable {
        /// Show the browser window on this PC instead of running headless
        /// (Chrome only — CloakBrowser always shows its window).
        #[arg(long, conflicts_with = "headless")]
        headed: bool,
        /// Run without a window (the default for Chrome).
        #[arg(long)]
        headless: bool,
        /// Quit a browser after this many minutes with no viewer or agent (0 = never; default 10).
        #[arg(long)]
        idle_minutes: Option<u32>,
        /// Use this Chrome-family binary (path or name on PATH) instead of auto-detecting
        /// (an empty value goes back to auto-detecting).
        #[arg(long)]
        path: Option<String>,
        /// Only these paired devices (id, id prefix ≥ 8 chars, or name). Replaces the list.
        #[arg(long)]
        device: Vec<String>,
        /// cloak (CloakBrowser, the default) or chrome.
        #[arg(long, value_parser = ["cloak", "chrome"])]
        engine: Option<String>,
        /// At most this many profiles' browsers at once (default 3).
        #[arg(long)]
        max_running: Option<u32>,
    },
    /// Turn it off (a running browser quits at its idle timeout or `serve` exit).
    Disable,
    /// Show the settings and which browser would be used.
    Status,
    /// Start (or reuse) a profile's browser in `unipi-host serve` and print its
    /// CDP endpoint (127.0.0.1 only) for agent-browser / Playwright.
    Start {
        /// The browser profile (default: `default`).
        #[arg(long)]
        profile: Option<String>,
        /// Recorded as the profile's last session.
        #[arg(long, hide = true)]
        session: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Point the stable agent address (`AGENT_BROWSER_CDP`) at a profile;
    /// with no profile, print the address and the active profile.
    Use {
        profile: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Stop a profile's browser (default: `default`).
    Stop {
        #[arg(long)]
        profile: Option<String>,
    },
    /// List the browser profiles: created, last used, cookie sites, running.
    Profiles {
        #[arg(long)]
        json: bool,
    },
    /// Create or remove a browser profile.
    Profile {
        #[command(subcommand)]
        action: ProfileAction,
    },
    /// Import your Chrome/Chromium/Brave cookies of one site into a
    /// CloakBrowser profile (UNI-294). Decrypted in this process (it has
    /// your D-Bus session / Keychain access); only the decrypted cookies
    /// and the domain cross control.sock to `unipi-host serve`, never
    /// printed or logged — counts and domain names only.
    Cookies {
        /// `chrome[:<profile dir>]` / `chromium[:…]` / `brave[:…]`
        /// (profile dir e.g. `Default`, `Profile 1`; default: the newest).
        #[arg(long, conflicts_with_all = ["from_dir", "list_sources"])]
        from: Option<String>,
        /// `<user data dir>[:<profile dir>]` instead of `--from`.
        #[arg(long, conflicts_with_all = ["from", "list_sources"])]
        from_dir: Option<String>,
        /// The site whose cookies to import (itself + subdomains).
        #[arg(long, required_unless_present = "list_sources")]
        domain: Option<String>,
        /// The CloakBrowser profile to import into (default: `default`).
        #[arg(long)]
        into: Option<String>,
        /// List detected browsers and their profile names; nothing secret.
        #[arg(long)]
        list_sources: bool,
        #[arg(long)]
        json: bool,
    },
}
#[derive(Subcommand)]
enum ProfileAction {
    /// Create a profile (a–z, 0–9, - and _, at most 40).
    Create { name: String },
    /// Remove a profile and everything in it (not `default`, not while running).
    Rm { name: String },
}
#[derive(Subcommand)]
enum ForwardAction {
    /// Turn forwarding on. Pick `--any` (any loopback port) or `--port` (repeatable).
    Enable {
        /// Allow any port listening on this PC's loopback.
        #[arg(long, conflicts_with = "port")]
        any: bool,
        /// Allow this port (repeat for more; adds to the existing list).
        #[arg(long)]
        port: Vec<u16>,
        /// With --port: replace the allowed ports instead of adding to them.
        #[arg(long, requires = "port")]
        only: bool,
        /// Only these paired devices (id, id prefix ≥ 8 chars, or name). Default: all paired.
        #[arg(long)]
        device: Vec<String>,
    },
    /// Turn forwarding off (the allow-list is kept for next time).
    Disable,
    /// Show the forwarding settings.
    Status,
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let paths = cli
        .data_dir
        .clone()
        .map(HostPaths::from_dir)
        .unwrap_or_else(HostPaths::resolve);
    // The data dir decides where a Windows `serve` logs (the installer's
    // logon task throws stderr away), so logging starts after it is known.
    unipi_host::logging::init(&paths, matches!(cli.command, Command::Serve { .. }));
    paths.ensure_dir()?;
    match cli.command {
        Command::Serve {
            relay,
            relay_only,
            mode,
            exit_with_parent,
            exclude_extension,
            no_extensions,
        } => {
            // UNI-228: one host per data dir (two on one secret.key make phones flap).
            let mode = mode
                .as_deref()
                .and_then(unipi_host::instance::Mode::parse)
                .unwrap_or_else(unipi_host::instance::Mode::detect);
            let _instance = match unipi_host::instance::acquire(&paths, mode) {
                Ok(lock) => lock,
                Err(error @ unipi_host::instance::AcquireError::Held(_)) => {
                    eprintln!("{error}");
                    std::process::exit(unipi_host::instance::EXIT_ALREADY_RUNNING);
                }
                Err(error) => return Err(error.into()),
            };
            if let Some(parent) = exit_with_parent {
                let paths = paths.clone();
                unipi_host::instance::exit_with_parent(parent, move || {
                    #[cfg(unix)]
                    let _ = std::fs::remove_file(unipi_host::push::socket_path(&paths));
                    #[cfg(not(unix))]
                    drop(paths);
                });
            }
            // H3 (spec-host §4.4): host-level safe mode. A start that exits within 60 s — or
            // dies before it recorded a success — is a failed start; after two consecutive
            // failed starts the next one runs with extensions off (providers on) and names
            // the suspects from host.log.
            let mut safe_mode = unipi_host::sessions::crash::HostSafeMode::load(&paths.root);
            safe_mode.begin_start();
            let safe_plan =
                safe_mode.plan(unipi_host::sessions::crash::suspects_from_log(&paths.root));
            if safe_plan.disable_extensions {
                tracing::warn!(
                    text = %unipi_host::sessions::crash::banner(&safe_plan),
                    "safe mode: starting without extensions"
                );
            }
            tracing::info!(mode = mode.as_str(), "taking the host lock");
            let key = unipi_host::secret::load_or_create(&paths.secret_key())?;
            let mut builder = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
                .secret_key(key)
                .alpns(vec![ALPN.to_vec()]);
            if relay != "n0" {
                anyhow::ensure!(relay.starts_with("https://"), "relay must be an https URL");
                builder = builder.relay_mode(iroh::RelayMode::Custom(iroh::RelayMap::from_iter([
                    relay.parse::<iroh::RelayUrl>()?,
                ])));
            }
            if relay_only {
                builder = builder.clear_ip_transports();
            }
            let endpoint = builder.bind().await?;
            unipi_host::endpoint::persist_and_watch(&endpoint, paths.endpoint()).await?;
            let paths_for_cleanup = paths.clone();
            // H3 (spec-host §4.2): one session hub for this host. Extensions are discovered the
            // way the `oino` binary does (distribution share dir, `~/.oino/extensions`, the
            // trust-gated project dir), run in Desktop mode, and reach the host services
            // through the process-wide wasm host hub.
            let agent_dir = unipi_host::sessions::services::agent_dir(&paths_for_cleanup.root);
            let cwd = std::env::current_dir().unwrap_or_else(|_| agent_dir.clone());
            let loader = unipi_host::sessions::services::HostLoader::build(
                &agent_dir,
                &cwd,
                no_extensions || safe_plan.disable_extensions,
                &exclude_extension,
            );
            for diagnostic in loader.diagnostics() {
                if diagnostic.error {
                    tracing::error!(message = %diagnostic.message, "extension discovery");
                } else {
                    tracing::warn!(message = %diagnostic.message, "extension discovery");
                }
            }
            // The process-wide extension hub, when extensions load at all (the cron run
            // notifications publish their bus event here too).
            let local_services = loader.local_services().cloned();
            let cron_agent_dir = agent_dir.clone();
            let flags = oino_core::services::ResourceFlags {
                no_extensions,
                safe_mode: safe_plan.disable_extensions.then(|| safe_plan.clone()),
                ..oino_core::services::ResourceFlags::default()
            };
            let hub =
                unipi_host::sessions::SessionHub::create(unipi_host::sessions::HostHubOptions {
                    cwd,
                    agent_dir,
                    docs_root: None,
                    session_dir: None,
                    mode: oino_core::extensions::api::ExtensionMode::Desktop,
                    has_ui: true,
                    approve: None,
                    flags,
                    loader: std::sync::Arc::new(loader),
                    models: None,
                    tools: oino_core::runtime::ToolSelection::Default,
                    exclude_tools: Vec::new(),
                })?;
            if safe_plan.disable_extensions {
                hub.notice(
                    unipi_host::sessions::NoticeLevel::Warning,
                    unipi_host::sessions::crash::banner(&safe_plan),
                );
            }
            // H4 (host.md §7.3–7.5): the protocol handler reaches the hub and its per-session
            // event feeds through this one service (`events_open`, the session streams).
            let _sessions =
                unipi_host::sessions::SessionService::install(std::sync::Arc::clone(&hub));
            // H8 (host.md §9.8; ext-abi §6): the cron scheduler is unipi-cron's, embedded
            // here as the single owner (`scheduler.lock`): it ticks every second, watches
            // `jobs/`, sends run notifications through the host's own E2E channels and wakes
            // linked chats through the hub. The lock also keeps a second host (or a CLI
            // that cannot reach this one) from firing the same jobs.
            let cron_paths = unipi_host::cron_serve::cron_paths_for_host(&paths_for_cleanup.root);
            let cron_scheduler = unipi_cron::Scheduler::with_paths(cron_paths.clone())
                .with_notify_sink(unipi_host::cron_notify::notify_sink(std::sync::Arc::new(
                    paths_for_cleanup.clone(),
                )));
            let cron_scheduler = unipi_host::cron_wake::install(
                cron_scheduler,
                std::sync::Arc::clone(&hub),
                cron_paths.clone(),
                local_services.clone(),
            );
            let cron_owner = match unipi_cron::embedded::Embedded::start_with(cron_scheduler) {
                Ok(owner) => Some(owner),
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "cron: this process is not the scheduler owner; \
                         job files stay editable but nothing fires"
                    );
                    None
                }
            };
            let cron_scheduler = cron_owner
                .as_ref()
                .map(|owner| std::sync::Arc::clone(&owner.scheduler));
            let cron = match &cron_scheduler {
                Some(scheduler) => unipi_host::cron_serve::CronHost::with_scheduler(
                    &paths_for_cleanup,
                    std::sync::Arc::clone(scheduler),
                ),
                None => unipi_host::cron_serve::CronHost::new(&paths_for_cleanup),
            };
            if let Some(local) = &local_services {
                let names = unipi_host::sessions::services::register_all(
                    local,
                    &unipi_host::sessions::services::HostServiceDeps {
                        data_dir: paths_for_cleanup.root.clone(),
                        agent_dir: cron_agent_dir.clone(),
                        host_name: hostname(),
                        mode: mode.as_str().to_owned(),
                        kanboard_bin: unipi_host::sessions::services::kanboard_bin(),
                        cron: cron_scheduler.as_ref().map(|scheduler| {
                            std::sync::Arc::new(unipi_cron::host_service::CronHostService::new(
                                std::sync::Arc::clone(scheduler),
                            ))
                                as std::sync::Arc<dyn oino_extension_api::host_service::HostService>
                        }),
                        // Registered by its own WP (`spec-browser` WP-B7).
                        browser: None,
                    },
                );
                tracing::info!(?names, "host services registered");
            }
            let handler = Handler {
                paths,
                host_id: endpoint.id(),
                host_name: hostname(),
                lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
                cron: cron.clone(),
            };
            // UNI-303: the stable agent address (AGENT_BROWSER_CDP) for this host.
            if let Err(error) =
                unipi_host::browser::agent_endpoint::serve(handler.paths.clone()).await
            {
                tracing::warn!(op = "browser_agent", %error, "no stable agent endpoint");
            }
            // UNI-214: local control socket (named pipe on Windows) for `unipi-host push`.
            let _control = unipi_host::push::serve_control(unipi_host::push::ControlContext {
                paths: handler.paths.clone(),
                hub: unipi_host::push::PushHub::shared(&handler.paths),
                host_name: handler.host_name.clone(),
                notify: true,
                cron,
            })
            .await?;
            // UNI-248: end time-limited approvals and expire stale requests.
            let sweeper = {
                let paths = handler.paths.clone();
                tokio::spawn(async move {
                    let mut tick = tokio::time::interval(std::time::Duration::from_secs(20));
                    loop {
                        tick.tick().await;
                        match unipi_host::sharing::sweep(&paths, unipi_host::push::now_ms()) {
                            Ok(changed) => {
                                for item in changed {
                                    tracing::info!(
                                        id = item.id,
                                        feature = item.feature.as_str(),
                                        status = item.status.as_str(),
                                        "feature request"
                                    );
                                }
                            }
                            Err(error) => tracing::warn!(%error, "feature request sweep failed"),
                        }
                    }
                })
            };
            // UNI-263 / v2 §9.7: look for a newer host every few hours.
            // `prompt` raises a notice for `events_open`; `auto` installs it and
            // ends this process — the app's supervisor starts the new binary on
            // its next backoff (no service to restart any more).
            let (restart_tx, restart_rx) = tokio::sync::oneshot::channel::<()>();
            let updater = {
                let paths = handler.paths.clone();
                tokio::spawn(async move {
                    if let Ok(exe) = std::env::current_exe() {
                        unipi_host::update::cleanup_old(&exe);
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                    loop {
                        if background_update(&paths, mode).await {
                            let _ = restart_tx.send(());
                            return;
                        }
                        tokio::time::sleep(unipi_host::update::CHECK_EVERY).await;
                    }
                })
            };
            // UNI-229 + v2: this PC's own UniPi app reaches the host here —
            // `app.sock` or the Windows named pipe, no iroh, works offline.
            // A local listener that can't start (another host of this user
            // owns the pipe name) is not fatal: iroh stays the transport
            // (spec §6.1 / risk R2).
            let local = match unipi_host::local::serve(handler.clone()) {
                Ok(listener) => Some(listener),
                Err(error) => {
                    tracing::warn!(
                        %error,
                        addr = %paths_for_cleanup.local_addr(),
                        "local listener unavailable; the host is reachable over iroh only"
                    );
                    None
                }
            };
            let router = iroh::protocol::Router::builder(endpoint)
                .accept(ALPN, handler)
                .spawn();
            tracing::info!(relay_only, "host ready");
            // spec-host §4.4: a start that stays up this long is a success and clears the
            // consecutive-failure counter.
            let safe_task = tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_secs(
                    unipi_host::sessions::crash::FAST_FAIL_SECS,
                ))
                .await;
                safe_mode.record_success(unipi_host::sessions::crash::FAST_FAIL_SECS);
            });
            // A self-update ended the loop early: the binary on disk is the new
            // one, so this process steps aside (the app supervisor respawns it).
            tokio::select! {
                result = shutdown_signal() => result?,
                _ = restart_rx => tracing::info!("binary replaced; exiting for a restart"),
            }
            router.shutdown().await?;
            if let Some(local) = local {
                local.abort();
            }
            unipi_host::local::cleanup(&paths_for_cleanup);
            sweeper.abort();
            updater.abort();
            if let Some(cron_owner) = cron_owner {
                // Stops ticking; running jobs are not killed (the next owner closes them).
                cron_owner.shutdown();
            }
            safe_task.abort();
            // Dispose the live sessions so extensions see `session_shutdown{quit}` and the
            // single-writer leases are released before this process goes away.
            if let Ok(cards) = hub.list().await {
                for card in cards {
                    if let Err(error) = hub.close(&card.session_id).await {
                        tracing::warn!(session_id = %card.session_id, %error, "closing a session");
                    }
                }
            }
            // UNI-215: never leave the dedicated browser running.
            unipi_host::browser::Manager::shutdown_all().await;
            unipi_host::browser::agent_endpoint::forget(&paths_for_cleanup.root);
            // host.md §9.3: terminals die with the host (they outlive nothing
            // but their own streams).
            unipi_host::term::TermRegistry::shared().kill_all();
            #[cfg(unix)]
            let _ = std::fs::remove_file(unipi_host::push::socket_path(&paths_for_cleanup));
            #[cfg(not(unix))]
            drop(paths_for_cleanup);
        }
        Command::Status { json } => {
            let status = unipi_host::instance::status(&paths)?;
            if json {
                println!("{}", serde_json::to_string(&status)?);
            } else {
                match &status {
                    unipi_host::instance::Status::Running { owner: Some(o) } => println!(
                        "running — pid {}, mode {}, version {}, since {}",
                        o.pid,
                        o.mode.as_str(),
                        o.version,
                        o.started_at
                    ),
                    unipi_host::instance::Status::Running { owner: None } => println!("running"),
                    unipi_host::instance::Status::NotRunning { .. } => {
                        println!("not running ({})", paths.root.display())
                    }
                }
            }
            if matches!(status, unipi_host::instance::Status::NotRunning { .. }) {
                std::process::exit(3);
            }
        }
        Command::Pair { ttl, json } => {
            let code = pair_command(&paths, &ttl, json).await?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Command::Devices => println!(
            "{}",
            serde_json::to_string_pretty(&DeviceStore::new(paths.devices()).list()?)?
        ),
        Command::Revoke { target } => {
            let store = DeviceStore::new(paths.devices());
            let matches: Vec<_> = store
                .list()?
                .into_iter()
                .filter(|d| d.id.starts_with(&target) || d.name == target)
                .collect();
            anyhow::ensure!(matches.len() == 1, "expected exactly one matching device");
            store.revoke(&matches[0].id.parse()?)?;
            println!("Revoked {}", matches[0].name);
        }
        Command::Forward { action } => forward_command(&paths, action)?,
        Command::Fs { action } => fs_command(&paths, action)?,
        Command::Browser { action } => browser_command(&paths, action).await?,
        Command::Requests { action } => requests_command(
            &paths,
            action.unwrap_or(RequestsAction::List { json: false }),
        )?,
        Command::Sharing {
            action: SharingAction::Status { json },
        } => sharing_status(&paths, json)?,
        Command::Update {
            check,
            exe,
            mode,
            json,
        } => update_command(&paths, check, exe, mode, json).await?,
        Command::Push {
            action: Some(action),
            ..
        } => push_admin(&paths, action).await?,
        Command::Push {
            action: None,
            file,
            device,
            note,
            name,
            no_wait,
        } => {
            let code = push_file(
                &paths,
                file.expect("clap requires FILE"),
                device,
                note,
                name,
                no_wait,
            )
            .await?;
            std::process::exit(code);
        }
        Command::Cron { action } => cron_command(&paths, action).await?,
    }
    Ok(())
}
async fn browser_command(paths: &HostPaths, action: BrowserAction) -> anyhow::Result<()> {
    use unipi_host::browser::{
        profiles, would_use, BrowserConfig, BrowserControl, Engine, INSTALL_HINT,
    };
    use unipi_host::sharing::{self, BrowserEnable};
    let path = paths.config();
    match action {
        BrowserAction::Enable {
            headed,
            headless,
            idle_minutes,
            path: binary,
            device,
            engine,
            max_running,
        } => {
            let headless = if headed {
                Some(false)
            } else if headless {
                Some(true)
            } else {
                None
            };
            let engine = engine.as_deref().and_then(Engine::parse);
            sharing::browser_enable(
                paths,
                BrowserEnable {
                    headless,
                    idle_minutes,
                    path: binary,
                    devices: (!device.is_empty()).then_some(device),
                    engine,
                    max_running,
                },
            )?;
        }
        BrowserAction::Disable => sharing::disable(paths, unipi_proto::FeatureKind::Browser)?,
        BrowserAction::Status => {}
        BrowserAction::Start {
            profile,
            session,
            json,
        } => {
            if let Some(name) = &profile {
                profiles::check_name(name)?;
            }
            let data = browser_control(paths, BrowserControl::Start { profile, session }).await?;
            let started: unipi_host::browser::StartedInfo = serde_json::from_value(data)?;
            if json {
                println!("{}", serde_json::to_string(&started)?);
            } else {
                println!(
                    "{} {} — profile {}{}\nCDP websocket: {}\nCDP http:      {}\nEngine: {} ({}){}\nStealth: {}",
                    if started.reused { "Reusing" } else { "Started" },
                    started.browser.name,
                    started.profile,
                    started.pid.map(|p| format!(", pid {p}")).unwrap_or_default(),
                    started.ws_url,
                    started.http.as_deref().unwrap_or("-"),
                    started.engine,
                    started.browser.version.as_deref().unwrap_or("?"),
                    if started.headless { ", headless" } else { ", window shown" },
                    if started.stealth { "yes".to_string() } else { format!("NO — {}", started.stealth_reason.as_deref().unwrap_or("plain Chrome")) },
                );
                // UNI-296: agents use the take-over aware endpoint (falls back to the raw port).
                if let Some(http) = started.agent_http.as_ref().or(started.http.as_ref()) {
                    if let Some(ws) = &started.agent_ws {
                        println!(
                            "Agent endpoint: {ws}  (pauses while the user takes over in the app)"
                        );
                    }
                    println!(
                        "Connect: agent-browser --cdp {} …   (or {http}/json/version)",
                        http.rsplit(':').next().unwrap_or("")
                    );
                }
                // UNI-303: the stable address agents get as AGENT_BROWSER_CDP.
                if let Some(stable) = unipi_host::browser::agent_endpoint::read(&paths.root) {
                    println!(
                        "Stable agent address: 127.0.0.1:{} (profile {}) — AGENT_BROWSER_CDP",
                        stable.port, stable.profile
                    );
                }
            }
            return Ok(());
        }
        BrowserAction::Use { profile, json } => {
            use unipi_host::browser::agent_endpoint;
            let endpoint = match profile {
                Some(name) => agent_endpoint::set_profile(&paths.root, &name)?,
                None => agent_endpoint::read(&paths.root).ok_or_else(|| {
                    anyhow::anyhow!(
                        "the host isn't running (no {}); start it first",
                        agent_endpoint::FILE
                    )
                })?,
            };
            if json {
                println!("{}", serde_json::to_string(&endpoint)?);
            } else {
                println!(
                    "Agent address: 127.0.0.1:{} (AGENT_BROWSER_CDP={}) — profile {}\nagent-browser commands drive the browser shown in the UniPi app.",
                    endpoint.port, endpoint.port, endpoint.profile
                );
            }
            return Ok(());
        }
        BrowserAction::Stop { profile } => {
            let data = browser_control(paths, BrowserControl::Stop { profile }).await?;
            let name = data["profile"].as_str().unwrap_or("default");
            if data["stopped"].as_bool() == Some(true) {
                println!("Stopped the browser of profile {name}.");
            } else {
                println!("Profile {name} had no browser running.");
            }
            return Ok(());
        }
        BrowserAction::Profiles { json } => {
            // From the serve when it runs (pids, viewers, agents), else from disk.
            let list: Vec<unipi_host::browser::ProfileInfo> =
                match browser_control(paths, BrowserControl::Profiles).await {
                    Ok(data) => serde_json::from_value(data)?,
                    Err(_) => unipi_host::browser::local_profiles(&paths.root),
                };
            if json {
                println!("{}", serde_json::to_string(&list)?);
                return Ok(());
            }
            if list.is_empty() {
                println!("No browser profiles yet (`default` is created on first start).");
            }
            let when = |ms: u64| {
                let ago = unipi_host::push::now_ms().saturating_sub(ms) / 60_000;
                if ago < 1 {
                    "just now".to_string()
                } else if ago < 120 {
                    format!("{ago} min ago")
                } else if ago < 48 * 60 {
                    format!("{} h ago", ago / 60)
                } else {
                    format!("{} days ago", ago / 1440)
                }
            };
            for p in &list {
                let state = match (p.running, p.pid) {
                    (true, Some(pid)) => format!(
                        "running (pid {pid}{}{}{})",
                        p.engine
                            .as_deref()
                            .map(|e| format!(", {e}"))
                            .unwrap_or_default(),
                        if p.stealth == Some(false) {
                            ", NOT stealthy"
                        } else {
                            ""
                        },
                        if p.viewers + p.cdp_clients > 0 {
                            format!(
                                ", in use: {} viewer(s), {} agent(s)",
                                p.viewers, p.cdp_clients
                            )
                        } else {
                            String::new()
                        }
                    ),
                    (true, None) => "running".to_string(),
                    _ => "stopped".to_string(),
                };
                println!(
                    "{:<16} {state}\n{:<16} created {}, last used {}{}\n{:<16} cookies: {}",
                    p.name,
                    "",
                    when(p.created),
                    p.last_used.map(when).unwrap_or_else(|| "never".into()),
                    p.last_session
                        .as_deref()
                        .map(|s| format!(" (session {s})"))
                        .unwrap_or_default(),
                    "",
                    if p.cookie_domains.is_empty() {
                        "none".to_string()
                    } else {
                        p.cookie_domains
                            .iter()
                            .map(|d| d.domain.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    },
                );
            }
            return Ok(());
        }
        BrowserAction::Profile {
            action: ProfileAction::Create { name },
        } => {
            let _ = profiles::migrate(&paths.root, unipi_host::push::now_ms());
            profiles::create(&paths.root, &name, unipi_host::push::now_ms())?;
            println!("Created browser profile {name} ({}).\nStart it: unipi-host browser start --profile {name}", profiles::dir(&paths.root, &name).display());
            return Ok(());
        }
        BrowserAction::Profile {
            action: ProfileAction::Rm { name },
        } => {
            profiles::check_name(&name)?;
            anyhow::ensure!(
                name != profiles::DEFAULT,
                "the default profile can't be removed"
            );
            let running = match browser_control(paths, BrowserControl::Profiles).await {
                Ok(data) => serde_json::from_value::<Vec<unipi_host::browser::ProfileInfo>>(data)?,
                Err(_) => unipi_host::browser::local_profiles(&paths.root),
            };
            if let Some(p) = running.iter().find(|p| p.name == name && p.running) {
                anyhow::bail!("profile {name} is running{} — stop it first: unipi-host browser stop --profile {name}", p.pid.map(|p| format!(" (pid {p})")).unwrap_or_default());
            }
            let home = unipi_host::fs::home_dir().ok();
            profiles::remove(&paths.root, &name, home.as_deref())?;
            println!("Removed browser profile {name}.");
            return Ok(());
        }
        BrowserAction::Cookies {
            from,
            from_dir,
            domain,
            into,
            list_sources,
            json,
        } => {
            cookies_command(paths, from, from_dir, domain, into, list_sources, json).await?;
            return Ok(());
        }
    }
    let config = BrowserConfig::load(&path)?;
    let (found, engine, stealth, reason) = would_use(&config);
    let devices = if config.devices.is_empty() {
        "all paired devices".to_string()
    } else {
        config.devices.join(", ")
    };
    let mode = match engine {
        Engine::Cloak => {
            let x = match unipi_host::browser::display::detect_backend_here() {
                Some(b) => format!(
                    "hidden display ({}) — shown only in the UniPi app",
                    b.program()
                ),
                None => format!("headless — {}", unipi_host::browser::HEADLESS_NOTE),
            };
            match unipi_host::browser::wanted_placement(
                &config,
                unipi_host::browser::can_show_window(),
            ) {
                unipi_host::browser::Placement::Window => {
                    "window shown on this PC (Show window is on)".to_string()
                }
                _ => x,
            }
        }
        _ if config.headless => "headless (no window on this PC)".to_string(),
        _ if unipi_host::browser::can_show_window() => {
            "headed (window visible on this PC)".to_string()
        }
        _ => "headed, but this session has no screen — runs headless".to_string(),
    };
    let binary = match (&found, engine) {
        (Some(b), Engine::Cloak) if !std::path::Path::new(&b.path).exists() => format!(
            "{} {} — downloaded on first start to {}",
            b.name,
            b.version.as_deref().unwrap_or(""),
            b.path
        ),
        (Some(b), _) => format!("{} ({})", b.name, b.path),
        (None, _) => format!("none found — {INSTALL_HINT}"),
    };
    println!(
        "Remote browser: {}\nEngine: {} (setting: {})\nStealth: {}\nBrowser: {binary}\nMode: {mode}\nIdle shutdown: {}\nAt most running: {}\nDevices: {devices}\nProfiles: {}\nConfig: {} (read on every request, no restart needed)",
        if config.is_enabled_for(unipi_host::protocol::LOCAL_DEVICE_ID) { "on" } else { "off" },
        engine.as_str(),
        config.engine.as_str(),
        if stealth { "yes".to_string() } else { format!("no — {}", reason.unwrap_or_default()) },
        if config.idle_minutes == 0 { "never".to_string() } else { format!("after {} min with no viewer or agent", config.idle_minutes) },
        config.max_running,
        profiles::root(&paths.root).display(),
        path.display()
    );
    Ok(())
}
/// `unipi-host browser cookies` (UNI-294): decrypted here (this process has
/// the user's D-Bus session / Keychain access), then the decrypted cookies
/// and the domain cross control.sock to the serve, which starts/reuses the
/// profile's browser and calls CDP `Storage.setCookies`. Never prints,
/// logs or returns a cookie value or a keyring secret.
async fn cookies_command(
    paths: &HostPaths,
    from: Option<String>,
    from_dir: Option<String>,
    domain: Option<String>,
    into: Option<String>,
    list_sources: bool,
    json: bool,
) -> anyhow::Result<()> {
    use unipi_host::browser::cookies::{self, DetectedSource};
    use unipi_host::browser::{profiles, BrowserControl, CookieWire};
    if list_sources {
        let home = unipi_host::fs::home_dir()
            .map_err(|e| anyhow::anyhow!(e.message.as_str().to_string()))?;
        let sources: Vec<DetectedSource> = cookies::detect_sources(&home);
        if json {
            println!("{}", serde_json::to_string(&sources)?);
            return Ok(());
        }
        if sources.is_empty() {
            println!("No Chrome-family browser with a cookie database was found on this PC.");
        }
        for s in &sources {
            println!(
                "{:<10} {}\n{:<10} profiles: {}",
                s.browser.as_str(),
                s.user_data_dir,
                "",
                s.profiles.join(", ")
            );
        }
        return Ok(());
    }
    let domain =
        domain.ok_or_else(|| anyhow::anyhow!("--domain is required (or pass --list-sources)"))?;
    anyhow::ensure!(
        !domain.trim().is_empty() && !domain.contains(char::is_whitespace),
        "bad --domain {domain:?}"
    );
    let into = into.unwrap_or_else(|| profiles::DEFAULT.into());
    profiles::check_name(&into)?;

    let home =
        unipi_host::fs::home_dir().map_err(|e| anyhow::anyhow!(e.message.as_str().to_string()))?;
    let resolved = match (&from, &from_dir) {
        (Some(spec), None) => cookies::resolve_from(spec, &home),
        (None, Some(spec)) => cookies::resolve_from_dir(spec),
        _ => Err("pass exactly one of --from or --from-dir".to_string()),
    }
    .map_err(|e| anyhow::anyhow!(e))?;

    let imported = cookies::run_import(&resolved.db_path, resolved.browser, &domain)
        .map_err(|e| anyhow::anyhow!(e))?;
    let count = imported.cookies.len();
    if count == 0 {
        println!(
            "No cookies found for {domain} in {} (skipped {}).",
            resolved.db_path.display(),
            imported.skipped
        );
        return Ok(());
    }

    let wire: Vec<CookieWire> = imported
        .cookies
        .iter()
        .map(|c| CookieWire {
            name: c.name.clone(),
            value: c.value.clone(),
            domain: c.domain.clone(),
            path: c.path.clone(),
            secure: c.secure,
            http_only: c.http_only,
            same_site: c.same_site.map(str::to_string),
            expires: c.expires,
        })
        .collect();
    let data = browser_control(
        paths,
        BrowserControl::Cookies {
            profile: into.clone(),
            domain: domain.clone(),
            cookies: wire,
        },
    )
    .await?;
    if json {
        println!(
            "{}",
            serde_json::to_string(
                &serde_json::json!({ "imported": count, "skipped": imported.skipped, "profile": into, "domain": domain })
            )?
        );
    } else {
        println!(
            "Imported {count} cookie(s) for {domain} into profile {into} ({} skipped, undecryptable or unsupported).",
            imported.skipped
        );
    }
    let _ = data;
    Ok(())
}
/// One `browser` request to the running serve over control.sock.
async fn browser_control(
    paths: &HostPaths,
    request: unipi_host::browser::BrowserControl,
) -> anyhow::Result<serde_json::Value> {
    use unipi_host::push::{ControlEvent, ControlRequest};
    let (mut lines, _w) =
        unipi_host::push::connect_control(paths, &ControlRequest::Browser { request }).await?;
    // A first start may download CloakBrowser (~200 MB): no timeout here.
    match lines.next_line().await? {
        Some(line) => match serde_json::from_str::<ControlEvent>(&line)? {
            ControlEvent::Browser { data } => Ok(data),
            ControlEvent::Error { message, hint } => anyhow::bail!("{message}{}", hint.map(|h| format!("\n{h}")).unwrap_or_default()),
            other => anyhow::bail!("unexpected reply from serve: {other:?}"),
        },
        None => anyhow::bail!("`unipi-host serve` closed the connection (an older serve without `browser start`? restart it)"),
    }
}
/// One `cron` request: to the running `serve` over `control.sock` when it owns the
/// scheduler, else straight against the job files. Storage ops work without a host
/// (ext-abi §6.5); the live ops (`run`/`stop`/…) then fail with the library's
/// "scheduler is not running" message instead of silently doing nothing.
async fn cron_control(
    paths: &HostPaths,
    request: unipi_cron::control::CronControl,
) -> anyhow::Result<serde_json::Value> {
    use unipi_host::push::{ControlEvent, ControlRequest};
    let connected = unipi_host::push::connect_control(
        paths,
        &ControlRequest::Cron {
            request: request.clone(),
        },
    )
    .await;
    let Ok((mut lines, _w)) = connected else {
        let cron = unipi_host::cron_serve::CronHost::new(paths);
        return cron.control(request).await.map_err(|e| anyhow::anyhow!(e));
    };
    match lines.next_line().await? {
        Some(line) => match serde_json::from_str::<ControlEvent>(&line)? {
            ControlEvent::Cron { data } => Ok(data),
            ControlEvent::Error { message, hint } => anyhow::bail!(
                "{message}{}",
                hint.map(|h| format!("\n{h}")).unwrap_or_default()
            ),
            other => anyhow::bail!("unexpected reply from serve: {other:?}"),
        },
        None => anyhow::bail!("`unipi-host serve` closed the connection"),
    }
}
/// `--schedule`/`--at` (plain language or machine form), resolved the way `add`/`edit` do
/// server-side (`unipi-cron`'s shared resolver: 6-field cron / `at:<RFC3339>`, a bare
/// RFC3339 instant accepted for `--at`).
fn resolve_schedule_input(
    input: &str,
    for_at: bool,
    tz_input: Option<&str>,
) -> anyhow::Result<String> {
    unipi_cron::control::resolve_schedule_input(input, for_at, tz_input)
        .map_err(|e| anyhow::anyhow!(e))
}

/// The schedule + next-3-fires line shown after `add`/`edit` (human mode).
fn schedule_summary(schedule: &str, tz_input: &str) -> String {
    use unipi_cron::schedule::{next_fires, resolve_tz, Schedule};
    let Ok(tz) = resolve_tz(tz_input) else {
        return format!("schedule: {schedule}");
    };
    let Ok(parsed) = Schedule::parse(schedule) else {
        return format!("schedule: {schedule}");
    };
    let fires = next_fires(&parsed, tz, chrono::Utc::now(), 3);
    if fires.is_empty() {
        return format!("schedule: {schedule} (no upcoming fire — already run, or in the past)");
    }
    let times = fires
        .iter()
        .map(|dt| {
            dt.with_timezone(&tz)
                .format("%Y-%m-%d %H:%M %Z")
                .to_string()
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("schedule: {schedule}\nnext: {times}")
}

/// Builds an `Option<NotifySpec>` from `--notify`/`--notify-on`: `"none"`
/// in either list clears that half back to the default; otherwise the
/// given values become `channels`/`events` (unset ones stay `None`,
/// i.e. default). `None` overall only when both lists are empty (the CLI
/// caller decides whether to even call this for `edit`, where "untouched"
/// and "cleared" are different).
fn notify_spec_from_flags(
    notify: &[String],
    notify_on: &[String],
) -> anyhow::Result<Option<unipi_cron::model::NotifySpec>> {
    let channels: Option<Vec<String>> = match notify {
        [] => None,
        v if v.iter().any(|c| c.eq_ignore_ascii_case("none")) => None,
        v => Some(v.to_vec()),
    };
    let events: Option<Vec<String>> = match notify_on {
        [] => None,
        v if v.iter().any(|c| c.eq_ignore_ascii_case("none")) => None,
        v => Some(v.to_vec()),
    };
    if channels.is_none() && events.is_none() {
        return Ok(None);
    }
    Ok(Some(unipi_cron::model::NotifySpec { channels, events }))
}

/// Builds an `Option<SessionLink>` from `--wake-session`/`--wake-here`.
/// `--wake-here` reads `$PI_SESSION_FILE` (pi's bash tool exports it —
/// `resolveSpawnContext` in pi's `bash.js`); `--wake-session none` (or
/// neither flag) clears the link.
fn session_link_from_flags(
    wake_session: Option<&str>,
    wake_here: bool,
) -> anyhow::Result<Option<unipi_cron::model::SessionLink>> {
    if wake_here {
        let file = std::env::var("PI_SESSION_FILE").map_err(|_| {
            anyhow::anyhow!("--wake-here needs $PI_SESSION_FILE, which pi's bash tool sets — run this from inside a pi session")
        })?;
        anyhow::ensure!(!file.is_empty(), "$PI_SESSION_FILE is empty");
        return Ok(Some(unipi_cron::model::SessionLink {
            session_file: Some(file),
            session_id: None,
        }));
    }
    match wake_session {
        None => Ok(None),
        Some(v) if v.eq_ignore_ascii_case("none") => Ok(None),
        Some(v) if v.ends_with(".jsonl") || v.contains('/') || v.contains('\\') => {
            Ok(Some(unipi_cron::model::SessionLink {
                session_file: Some(v.to_string()),
                session_id: None,
            }))
        }
        Some(v) => Ok(Some(unipi_cron::model::SessionLink {
            session_file: None,
            session_id: Some(v.to_string()),
        })),
    }
}

async fn cron_command(paths: &HostPaths, action: CronAction) -> anyhow::Result<()> {
    use unipi_cron::control::{CronControl, EditJob, NewJob};
    // `<agent dir>/cron` — the same files `serve`'s embedded scheduler and the extension use.
    let cron_paths = unipi_host::cron_serve::cron_paths_for_host(&paths.root);
    let print_or = |json: bool, value: &serde_json::Value, text: String| {
        if json {
            println!("{}", serde_json::to_string(value).unwrap_or_default());
        } else {
            println!("{text}");
        }
    };
    match action {
        CronAction::List { json } => {
            let data = cron_control(paths, CronControl::List).await?;
            if json {
                println!("{}", serde_json::to_string(&data)?);
                return Ok(());
            }
            let jobs = data["jobs"].as_array().cloned().unwrap_or_default();
            if jobs.is_empty() {
                println!(
                    "No cron jobs yet. Add one: unipi-host cron add --name \"daily digest\" --prompt \"Summarize today's inbox\" --schedule \"every day at 9:00\""
                );
            }
            for job in &jobs {
                println!(
                    "{:<20} {:<8} {}  {}  {}",
                    job["id"].as_str().unwrap_or(""),
                    if job["enabled"].as_bool().unwrap_or(false) {
                        "enabled"
                    } else {
                        "disabled"
                    },
                    job["schedule"].as_str().unwrap_or(""),
                    job["tz"].as_str().unwrap_or(""),
                    job["name"].as_str().unwrap_or(""),
                );
            }
        }
        CronAction::Show { id, json } => {
            let data = cron_control(paths, CronControl::Show { id }).await?;
            if json {
                println!("{}", serde_json::to_string(&data)?);
                return Ok(());
            }
            let job = &data["job"];
            let tz = job["tz"].as_str().unwrap_or("UTC");
            let mut lines = vec![
                format!(
                    "{} — {}",
                    job["id"].as_str().unwrap_or(""),
                    job["name"].as_str().unwrap_or("")
                ),
                format!(
                    "  {}",
                    if job["enabled"].as_bool().unwrap_or(false) {
                        "enabled"
                    } else {
                        "disabled"
                    }
                ),
                format!("  prompt: {}", job["prompt"].as_str().unwrap_or("")),
                format!("  cwd: {}", job["cwd"].as_str().unwrap_or("")),
                schedule_summary(job["schedule"].as_str().unwrap_or(""), tz)
                    .lines()
                    .map(|l| format!("  {l}"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ];
            if let Some(model) = job["model"].as_str() {
                lines.push(format!("  model: {model}"));
            }
            if let Some(cap) = job["cost_cap_usd"].as_f64() {
                lines.push(format!("  cost cap: ${cap:.2}"));
            }
            lines.push(String::new());
            lines.push("runs:".to_string());
            let runs = data["runs"].as_array().cloned().unwrap_or_default();
            if runs.is_empty() {
                lines.push("  none yet".to_string());
            }
            for run in runs.iter().take(10) {
                lines.push(format!(
                    "  {:<8} {:<8} {}",
                    run["id"].as_str().unwrap_or(""),
                    run["status"].as_str().unwrap_or(""),
                    run["summary"]
                        .as_str()
                        .unwrap_or(run["reason"].as_str().unwrap_or("")),
                ));
            }
            println!("{}", lines.join("\n"));
        }
        CronAction::Add {
            id,
            name,
            prompt,
            schedule,
            at,
            tz,
            cwd,
            model,
            thinking,
            disabled,
            time_limit_s,
            cost_cap,
            notify,
            notify_on,
            wake_session,
            wake_here,
            json,
        } => {
            let raw_schedule = schedule.or(at.clone()).ok_or_else(|| {
                anyhow::anyhow!("cron add needs --schedule (a recurring schedule) or --at (one-time) — example: --schedule \"every day at 9:00\" or --at \"in 2 hours\"")
            })?;
            let resolved_schedule =
                resolve_schedule_input(&raw_schedule, at.is_some(), tz.as_deref())?;
            let cwd = cwd.unwrap_or_else(|| ".".to_string());
            let cwd = std::fs::canonicalize(&cwd)
                .map_err(|e| anyhow::anyhow!("--cwd {cwd:?}: {e}"))?
                .to_string_lossy()
                .into_owned();
            let notify_spec = notify_spec_from_flags(&notify, &notify_on)?;
            let session_link = session_link_from_flags(wake_session.as_deref(), wake_here)?;
            let data = cron_control(
                paths,
                CronControl::Add(Box::new(NewJob {
                    id,
                    name,
                    prompt,
                    schedule: resolved_schedule,
                    tz: tz.clone(),
                    cwd: Some(cwd),
                    model,
                    thinking,
                    enabled: Some(!disabled),
                    time_limit_s,
                    cost_cap_usd: cost_cap,
                    overlap: None,
                    missed: None,
                    notify: notify_spec,
                    session_link,
                })),
            )
            .await?;
            let job_id = data["id"].as_str().unwrap_or("").to_string();
            let job_tz = data["tz"].as_str().unwrap_or("UTC").to_string();
            let job_schedule = data["schedule"].as_str().unwrap_or("").to_string();
            print_or(
                json,
                &data,
                format!(
                    "Added job {job_id}.\n{}",
                    schedule_summary(&job_schedule, &job_tz)
                ),
            );
        }
        CronAction::Edit {
            id,
            name,
            prompt,
            schedule,
            at,
            tz,
            cwd,
            model,
            thinking,
            time_limit_s,
            cost_cap,
            notify,
            notify_on,
            wake_session,
            wake_here,
            json,
        } => {
            let resolved_schedule = match schedule.or(at.clone()) {
                Some(raw) => Some(resolve_schedule_input(&raw, at.is_some(), tz.as_deref())?),
                None => None,
            };
            let cwd = cwd
                .map(|c| {
                    std::fs::canonicalize(&c)
                        .map(|p| p.to_string_lossy().into_owned())
                        .map_err(|e| anyhow::anyhow!("--cwd {c:?}: {e}"))
                })
                .transpose()?;
            let notify_patch = if notify.is_empty() && notify_on.is_empty() {
                None
            } else {
                Some(notify_spec_from_flags(&notify, &notify_on)?)
            };
            let session_link_patch = if wake_session.is_none() && !wake_here {
                None
            } else {
                Some(session_link_from_flags(wake_session.as_deref(), wake_here)?)
            };
            let patch = EditJob {
                name,
                prompt,
                schedule: resolved_schedule,
                tz,
                cwd,
                model: model.map(Some),
                thinking: thinking.map(Some),
                enabled: None,
                time_limit_s,
                cost_cap_usd: cost_cap.map(Some),
                overlap: None,
                missed: None,
                notify: notify_patch,
                session_link: session_link_patch,
            };
            let data = cron_control(
                paths,
                CronControl::Edit {
                    id: id.clone(),
                    patch: Box::new(patch),
                },
            )
            .await?;
            let job_tz = data["tz"].as_str().unwrap_or("UTC").to_string();
            let job_schedule = data["schedule"].as_str().unwrap_or("").to_string();
            print_or(
                json,
                &data,
                format!(
                    "Updated job {id}.\n{}",
                    schedule_summary(&job_schedule, &job_tz)
                ),
            );
        }
        CronAction::TestNotify { id, json } => {
            let data = cron_control(paths, CronControl::TestNotify { id: id.clone() }).await?;
            if json {
                println!("{}", serde_json::to_string(&data)?);
                return Ok(());
            }
            let channels = data["channels"].as_array().cloned().unwrap_or_default();
            let names: Vec<String> = channels
                .iter()
                .filter_map(|c| c.as_str().map(str::to_string))
                .collect();
            let errors = data["errors"].as_array().cloned().unwrap_or_default();
            if errors.is_empty() {
                println!("Sent a test notification on: {}", names.join(", "));
            } else {
                println!("Tried: {}", names.join(", "));
                for error in &errors {
                    println!("  {}", error.as_str().unwrap_or(""));
                }
            }
        }
        CronAction::Rm { id } => {
            let data = cron_control(paths, CronControl::Remove { id: id.clone() }).await?;
            println!("Removed {}", data["removed"].as_str().unwrap_or(""));
        }
        CronAction::Enable { id } => {
            cron_control(paths, CronControl::Enable { id: id.clone() }).await?;
            println!("Enabled {id}");
        }
        CronAction::Disable { id } => {
            cron_control(paths, CronControl::Disable { id: id.clone() }).await?;
            println!("Disabled {id}");
        }
        CronAction::Run { id, json } => {
            let data = cron_control(paths, CronControl::RunNow { id }).await?;
            print_or(
                json,
                &data,
                format!("Started run {}", data["run_id"].as_str().unwrap_or("")),
            );
        }
        CronAction::Stop { id } => {
            let data = cron_control(paths, CronControl::Stop { id }).await?;
            println!(
                "{}",
                if data["stopped"].as_bool() == Some(true) {
                    "Stopped."
                } else {
                    "Nothing was running."
                }
            );
        }
        CronAction::Runs { id, limit, json } => {
            let data = cron_control(paths, CronControl::Runs { job_id: id, limit }).await?;
            if json {
                println!("{}", serde_json::to_string(&data)?);
                return Ok(());
            }
            let runs = data["runs"].as_array().cloned().unwrap_or_default();
            if runs.is_empty() {
                println!("No runs yet.");
            }
            for run in &runs {
                println!(
                    "{:<10} {:<8} {:<8} {}",
                    run["job_id"].as_str().unwrap_or(""),
                    run["id"].as_str().unwrap_or(""),
                    run["status"].as_str().unwrap_or(""),
                    run["summary"]
                        .as_str()
                        .unwrap_or(run["reason"].as_str().unwrap_or("")),
                );
            }
        }
        CronAction::Log { run_id, json } => {
            let data = cron_control(paths, CronControl::Log { run_id }).await?;
            if json {
                println!("{}", serde_json::to_string(&data)?);
                return Ok(());
            }
            let run = &data["run"];
            println!(
                "{} [{}] job {}  exit {}  session {}  cost {}",
                run["id"].as_str().unwrap_or(""),
                run["status"].as_str().unwrap_or(""),
                data["job_id"].as_str().unwrap_or(""),
                run["exit_code"]
                    .as_i64()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                run["session_path"].as_str().unwrap_or("-"),
                run["cost_usd"]
                    .as_f64()
                    .map(|c| format!("${c:.4}"))
                    .unwrap_or_else(|| "-".to_string()),
            );
            println!("{}", data["log"].as_str().unwrap_or(""));
        }
        CronAction::Pause => {
            cron_control(paths, CronControl::PauseAll).await?;
            println!("Paused: no new runs will start. Running ones finish.");
        }
        CronAction::Resume => {
            cron_control(paths, CronControl::ResumeAll).await?;
            println!("Resumed.");
        }
        CronAction::Status { json } => {
            let data = cron_control(paths, CronControl::Status).await?;
            if json {
                println!("{}", serde_json::to_string(&data)?);
                return Ok(());
            }
            let mut lines = vec![format!(
                "{}  concurrency {}",
                if data["paused"].as_bool().unwrap_or(false) {
                    "paused"
                } else {
                    "running"
                },
                data["concurrency"].as_i64().unwrap_or(0),
            )];
            for job in data["jobs"].as_array().cloned().unwrap_or_default() {
                lines.push(format!(
                    "  {:<20} {}  next {}  {}",
                    job["id"].as_str().unwrap_or(""),
                    if job["enabled"].as_bool().unwrap_or(false) {
                        "enabled "
                    } else {
                        "disabled"
                    },
                    job["next_fire"].as_str().unwrap_or("-"),
                    if job["running"].as_bool().unwrap_or(false) {
                        "(running)"
                    } else {
                        ""
                    },
                ));
            }
            println!("{}", lines.join("\n"));
        }
        CronAction::Share { device } => {
            unipi_host::sharing::cron_enable(paths, (!device.is_empty()).then_some(device))?;
            let settings = unipi_cron::store::load_settings(&cron_paths)?;
            print_cron_settings(&settings, &unipi_host::sharing::cron_share(paths));
        }
        CronAction::Unshare => {
            unipi_host::sharing::disable(paths, unipi_proto::FeatureKind::Cron)?;
            let settings = unipi_cron::store::load_settings(&cron_paths)?;
            print_cron_settings(&settings, &unipi_host::sharing::cron_share(paths));
        }
        CronAction::Settings {
            concurrency,
            missed_window_s,
            retention_runs,
            retention_days,
            default_time_limit_s,
            paused,
            notify_default_events,
            json,
        } => {
            let any_set = concurrency.is_some()
                || missed_window_s.is_some()
                || retention_runs.is_some()
                || retention_days.is_some()
                || default_time_limit_s.is_some()
                || paused.is_some()
                || notify_default_events.is_some();
            let settings = if any_set {
                let patch = unipi_proto::CronSettingsSetRequest {
                    concurrency,
                    missed_window_s,
                    retention_runs,
                    retention_days,
                    default_time_limit_s,
                    paused,
                    ntfy_topic: None,
                    notify_default_events: notify_default_events.map(|s| {
                        s.split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect()
                    }),
                };
                unipi_host::cron_serve::apply_settings_patch(&cron_paths, patch)
                    .map_err(|e| anyhow::anyhow!(e))?
            } else {
                unipi_cron::store::load_settings(&cron_paths)?
            };
            if json {
                println!("{}", serde_json::to_string(&settings)?);
                return Ok(());
            }
            print_cron_settings(&settings, &unipi_host::sharing::cron_share(paths));
        }
    }
    Ok(())
}
fn print_cron_settings(
    settings: &unipi_cron::model::CronSettings,
    share: &unipi_host::sharing::CronShareConfig,
) {
    println!(
        "concurrency {}  missed_window_s {}  retention_runs {}  retention_days {}  default_time_limit_s {}",
        settings.concurrency, settings.missed_window_s, settings.retention_runs, settings.retention_days, settings.default_time_limit_s,
    );
    println!(
        "paused {}  cron shared with phones: {}",
        settings.paused, share.enabled
    );
    if share.enabled {
        println!(
            "  devices: {}",
            if share.devices.is_empty() {
                "all paired devices".to_string()
            } else {
                share.devices.join(", ")
            }
        );
    }
    println!(
        "notify_default_events {}",
        if settings.notify_default_events.is_empty() {
            "-".to_string()
        } else {
            settings.notify_default_events.join(",")
        }
    );
}
fn fs_command(paths: &HostPaths, action: FsAction) -> anyhow::Result<()> {
    use unipi_host::fs_scope::{allow, deny, Edit, FsAccessConfig, FsScope};
    let path = paths.config();
    let home =
        unipi_host::fs::home_dir().map_err(|e| anyhow::anyhow!(e.message.as_str().to_string()))?;
    let mut config = FsAccessConfig::load(&path)?;
    let edit = match &action {
        FsAction::Allow { path: p } => Some(allow(&mut config, p, &home)?),
        FsAction::Deny { path: p } => Some(deny(&mut config, p, &home)?),
        FsAction::List => None,
    };
    match &edit {
        Some(Edit::Changed(message)) => {
            config.save(&path)?;
            println!("{message}");
        }
        Some(Edit::Unchanged(message)) => println!("{message}"),
        None => {}
    }
    let scope = FsScope::resolve(home.clone(), &config);
    let list = |v: &[String]| {
        if v.is_empty() {
            "none".to_string()
        } else {
            v.join(", ")
        }
    };
    println!(
        "The app can open: {} (home), session folders{}\nAllowed (fs allow): {}\nDenied (fs deny): {}\nConfig: {} (read on every request, no restart needed)",
        home.display(),
        scope.extra.iter().map(|r| format!(", {}", r.display())).collect::<String>(),
        list(&config.allow),
        list(&config.deny),
        path.display()
    );
    Ok(())
}
fn forward_command(paths: &HostPaths, action: ForwardAction) -> anyhow::Result<()> {
    use unipi_host::forward::{HostConfig, PortRule};
    use unipi_host::sharing::{self, ForwardEnable};
    let path = paths.config();
    match action {
        ForwardAction::Enable {
            any,
            port,
            only,
            device,
        } => {
            sharing::forward_enable(
                paths,
                ForwardEnable {
                    any,
                    ports: port,
                    replace: only,
                    devices: (!device.is_empty()).then_some(device),
                },
            )?;
        }
        ForwardAction::Disable => sharing::disable(paths, unipi_proto::FeatureKind::Forward)?,
        ForwardAction::Status => {}
    }
    let config = HostConfig::load(&path)?;
    let f = &config.forwarding;
    let ports = match &f.ports {
        PortRule::Any => "any loopback port".to_string(),
        PortRule::None => "none".to_string(),
        PortRule::List(list) => list
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", "),
    };
    let devices = if f.devices.is_empty() {
        "all paired devices".to_string()
    } else {
        f.devices.join(", ")
    };
    println!(
        "Port forwarding: {}\nPorts: {ports}\nDevices: {devices}\nConfig: {} (read on every request, no restart needed)",
        if f.enabled { "on" } else { "off" },
        path.display()
    );
    Ok(())
}
/// `unipi-host pair [--json]` (UNI-266). Exit code: 0 paired / expired, 1 error (JSON mode).
async fn pair_command(paths: &HostPaths, ttl: &str, json: bool) -> anyhow::Result<i32> {
    use std::io::Write;
    use unipi_host::pair_cli::{self, PairEvent};
    let emit = |event: &PairEvent| {
        let mut out = std::io::stdout().lock();
        let _ = writeln!(out, "{}", serde_json::to_string(event).unwrap_or_default());
        let _ = out.flush();
    };
    let started = match pair_cli::parse_ttl(ttl) {
        Ok(seconds) => pair_cli::start(paths, seconds, &hostname()).await,
        Err(e) => Err(e),
    };
    let started = match started {
        Ok(started) => started,
        Err(error) if json => {
            let message = error.to_string();
            let hint =
                (message == pair_cli::NOT_RUNNING).then(|| pair_cli::start_hint().to_string());
            emit(&PairEvent::Error { message, hint });
            return Ok(1);
        }
        Err(error) if error.to_string() == pair_cli::NOT_RUNNING => {
            anyhow::bail!("{error}\n{}", pair_cli::start_hint())
        }
        Err(error) => return Err(error),
    };
    if json {
        emit(&started.event());
        let event = pair_cli::wait(paths, &started.state, pair_cli::stdin_closed()).await?;
        emit(&event);
        return Ok(0);
    }
    let qr = unipi_host::pair_link::render_terminal_qr(&started.link)?;
    use std::io::IsTerminal;
    let color = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    println!(
        "{}\n\nScan with the UniPi app (Add a host → Pair with QR), or paste this link:\n{}",
        qr.to_terminal_string(color),
        started.link
    );
    tracing::debug!(version = qr.version, columns = qr.columns(), "pairing QR");
    match pair_cli::wait(paths, &started.state, std::future::pending()).await? {
        PairEvent::Paired { device } => println!("Paired with {device}"),
        _ => println!("Expired"),
    }
    Ok(0)
}
fn hostname() -> String {
    hostname::get()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "UniPi host".into())
        .chars()
        .take(64)
        .collect()
}
/// Ctrl+C or SIGTERM (systemd stop), so the control socket is removed.
async fn shutdown_signal() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r,
            _ = term.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}
fn print_push_status(paths: &HostPaths, config: &unipi_host::push::PushConfig) {
    let devices = if config.devices.is_empty() {
        "all paired devices".to_string()
    } else {
        config.devices.join(", ")
    };
    println!(
        "Push to phone: {}\nDevices: {devices}\nConfig: {} (read on every request, no restart needed)",
        if config.enabled { "on" } else { "off" },
        paths.config().display()
    );
}
async fn push_admin(paths: &HostPaths, action: PushAction) -> anyhow::Result<()> {
    use unipi_host::forward::HostConfig;
    use unipi_host::push::{ControlEvent, ControlRequest};
    let path = paths.config();
    let mut config = HostConfig::load(&path)?;
    match action {
        PushAction::Enable { device } => {
            let push =
                unipi_host::sharing::push_enable(paths, (!device.is_empty()).then_some(device))?;
            print_push_status(paths, &push);
        }
        PushAction::Disable => {
            unipi_host::sharing::disable(paths, unipi_proto::FeatureKind::Push)?;
            config = HostConfig::load(&path)?;
            print_push_status(paths, &config.push);
        }
        PushAction::Status => {
            print_push_status(paths, &config.push);
            match unipi_host::push::connect_control(paths, &ControlRequest::List).await {
                Ok((mut lines, _w)) => {
                    if let Some(line) = lines.next_line().await? {
                        if let Ok(ControlEvent::Pending { pushes }) = serde_json::from_str(&line) {
                            if pushes.is_empty() {
                                println!("Pending: none");
                            }
                            for p in pushes {
                                println!(
                                    "Pending: {}  {} ({})  waiting for {} device(s)",
                                    p.item.id.as_str(),
                                    p.item.name,
                                    unipi_host::push::human_size(p.item.size),
                                    p.remaining.len()
                                );
                            }
                        }
                    }
                }
                Err(_) => println!("Host: not running (start `unipi-host serve` to push)"),
            }
        }
        PushAction::Cancel { id } => {
            let (mut lines, _w) =
                unipi_host::push::connect_control(paths, &ControlRequest::Cancel { id }).await?;
            if let Some(line) = lines.next_line().await? {
                if let Ok(ControlEvent::Cancelled { ids }) = serde_json::from_str(&line) {
                    println!(
                        "Cancelled {} push(es){}",
                        ids.len(),
                        if ids.is_empty() {
                            String::new()
                        } else {
                            format!(": {}", ids.join(", "))
                        }
                    );
                }
            }
        }
    }
    Ok(())
}
/// `unipi-host push <file>`: exit 0 when a phone installed/saved/opened it
/// (or queued with --no-wait), 1 on errors/failure, 2 when dismissed.
async fn push_file(
    paths: &HostPaths,
    file: std::path::PathBuf,
    device: Vec<String>,
    note: Option<String>,
    name: Option<String>,
    no_wait: bool,
) -> anyhow::Result<i32> {
    use unipi_host::push::{human_size, ControlEvent, ControlRequest};
    use unipi_proto::PushOutcome;
    let file = std::fs::canonicalize(&file)
        .map_err(|e| anyhow::anyhow!("can't read {}: {e}", file.display()))?;
    anyhow::ensure!(file.is_file(), "{} is not a file", file.display());
    // Explain the gate before hashing a big file.
    let config = unipi_host::forward::HostConfig::load(&paths.config())?.push;
    if !config.enabled {
        eprintln!(
            "Push is off on this PC (the default).\n{}",
            unipi_host::push::ENABLE_HINT
        );
        eprintln!("Then run this command again.");
        return Ok(1);
    }
    let size = std::fs::metadata(&file)?.len();
    anyhow::ensure!(size > 0, "{} is empty", file.display());
    anyhow::ensure!(
        size <= unipi_proto::BLOB_MAX_SIZE,
        "{} is {} — the phone accepts at most {}",
        file.display(),
        human_size(size),
        human_size(unipi_proto::BLOB_MAX_SIZE)
    );
    let hashed = file.clone();
    let (sha256, size) =
        tokio::task::spawn_blocking(move || unipi_host::push::sha256_file(&hashed)).await??;
    {
        let request = ControlRequest::Push {
            path: file.to_string_lossy().into_owned(),
            name,
            note,
            devices: device,
            sha256,
            size,
            follow: !no_wait,
        };
        let (mut lines, _keep_open) = unipi_host::push::connect_control(paths, &request).await?;
        let shown = file
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let mut code = 1;
        let mut answered = 0usize;
        let mut targets = 0usize;
        while let Some(line) = lines.next_line().await? {
            let Ok(event) = serde_json::from_str::<ControlEvent>(&line) else {
                continue;
            };
            match event {
                ControlEvent::Error { message, hint } => {
                    eprintln!("{message}");
                    if let Some(hint) = hint {
                        eprintln!("{hint}");
                    }
                    return Ok(1);
                }
                ControlEvent::Queued {
                    id,
                    devices,
                    notified,
                } => {
                    targets = devices.len();
                    println!(
                        "Sent {shown} ({}) to {} — id {id}",
                        human_size(size),
                        devices.join(", ")
                    );
                    if notified {
                        println!("The phone wasn't open, so a notification was sent too.");
                    }
                    if no_wait {
                        println!("Not waiting (--no-wait). `unipi-host push status` shows what's pending.");
                        return Ok(0);
                    }
                    println!("Waiting for the phone… (Ctrl+C stops waiting; the push stays pending for 24 h)");
                }
                ControlEvent::Fetching { device, .. } => println!("{device}: downloading…"),
                ControlEvent::Done {
                    device,
                    outcome,
                    detail,
                    ..
                } => {
                    answered += 1;
                    let what = match outcome {
                        PushOutcome::Installed => "opened the installer",
                        PushOutcome::Saved => "saved it",
                        PushOutcome::Opened => "opened it",
                        PushOutcome::Dismissed => "dismissed it",
                        PushOutcome::Failed => "couldn't use it",
                    };
                    println!(
                        "{device}: {what}{}",
                        detail.map(|d| format!(" ({d})")).unwrap_or_default()
                    );
                    code = match outcome {
                        PushOutcome::Installed | PushOutcome::Saved | PushOutcome::Opened => 0,
                        PushOutcome::Dismissed if code != 0 => 2,
                        _ => code,
                    };
                }
                ControlEvent::Finished { .. } => {
                    if answered < targets {
                        println!("Cancelled or expired before every phone answered.");
                    }
                    return Ok(code);
                }
                _ => {}
            }
        }
        eprintln!("`unipi-host serve` stopped; the push is kept and resumes when it starts again.");
        Ok(1)
    }
}

fn requests_command(paths: &HostPaths, action: RequestsAction) -> anyhow::Result<()> {
    use unipi_host::sharing;
    use unipi_proto::FeatureRequestStatus;
    let now = unipi_host::push::now_ms();
    match action {
        RequestsAction::List { json } => {
            let mut list = sharing::all(paths, now)?;
            if json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&sharing::status(paths, now)?.requests)?
                );
                return Ok(());
            }
            list.sort_by_key(|r| {
                (
                    r.item.status != FeatureRequestStatus::Pending,
                    std::cmp::Reverse(r.item.created_at),
                )
            });
            if list.is_empty() {
                println!(
                    "No requests. Phones can ask from Preview when a feature is off on this PC."
                );
            }
            for r in &list {
                let age = now.saturating_sub(r.item.created_at) / 60_000;
                let when = if age < 1 {
                    "just now".to_string()
                } else if age < 60 {
                    format!("{age} min ago")
                } else {
                    format!("{} h ago", age / 60)
                };
                let until = r
                    .item
                    .until
                    .map(|u| format!(" until {} min from now", u.saturating_sub(now) / 60_000))
                    .unwrap_or_default();
                println!(
                    "{}  {:<8}  {}  ({when}){until}",
                    r.item.id,
                    r.item.status.as_str(),
                    sharing::describe(&r.item)
                );
                if let Some(note) = &r.item.note {
                    println!("{:>12}  \"{note}\"", "");
                }
            }
            if list
                .iter()
                .any(|r| r.item.status == FeatureRequestStatus::Pending)
            {
                println!("\nApprove: unipi-host requests approve <id> [--for 1h]   Deny: unipi-host requests deny <id>");
            }
        }
        RequestsAction::Approve { id, for_ } => {
            let until = match for_.as_deref() {
                None | Some("off") | Some("forever") => None,
                Some(text) => Some(
                    now + sharing::parse_duration_ms(text).ok_or_else(|| {
                        anyhow::anyhow!("--for takes a duration like 1h, 30m or 2d")
                    })?,
                ),
            };
            let item = sharing::approve(paths, &id, until, now)?;
            println!(
                "Approved: {} is on for {}{}. No restart needed.",
                item.feature.label(),
                item.device_name,
                until
                    .map(|u| format!(" for the next {} min", (u - now) / 60_000))
                    .unwrap_or_else(|| " until you turn it off".into())
            );
        }
        RequestsAction::Deny { id } => {
            let item = sharing::deny(paths, &id, now)?;
            println!(
                "Denied {}'s request for {}.",
                item.device_name,
                item.feature.label()
            );
        }
    }
    Ok(())
}
fn sharing_status(paths: &HostPaths, json: bool) -> anyhow::Result<()> {
    let status = unipi_host::sharing::status(paths, unipi_host::push::now_ms())?;
    if json {
        println!("{}", serde_json::to_string(&status)?);
        return Ok(());
    }
    let onoff = |b: bool| if b { "on" } else { "off" };
    println!(
        "Web apps (forward): {}{}\nBrowser: {}{}\nPush: {}\nPending requests: {}",
        onoff(status.forward.enabled),
        if status.forward.any {
            " · any port".to_string()
        } else if status.forward.ports.is_empty() {
            String::new()
        } else {
            format!(" · ports {:?}", status.forward.ports)
        },
        onoff(status.browser.enabled),
        if status.browser.headless {
            " · headless"
        } else {
            " · window shown"
        },
        onoff(status.push.enabled),
        status
            .requests
            .iter()
            .filter(|r| r.item.status == unipi_proto::FeatureRequestStatus::Pending)
            .count()
    );
    Ok(())
}
/// One background check from `serve` (UNI-263, host.md §9.7). In `prompt` mode
/// (the default) it only records the check and raises a [`update::Notice`] for
/// `events_open`; `auto` installs the newer host and answers `true` — the
/// caller then ends this process (no service to restart any more).
async fn background_update(paths: &HostPaths, instance_mode: unipi_host::instance::Mode) -> bool {
    use unipi_host::update;
    let result = update::fetch_feed().await;
    update::record(paths, now_unix(), &result);
    let Ok(release) = result else { return false };
    let setting = update::UpdateConfig::load(&paths.config())
        .map(|c| c.mode)
        .unwrap_or_default();
    let decision = update::decide(
        &release,
        env!("CARGO_PKG_VERSION"),
        &update::current_target(),
    );
    if setting != update::UpdateMode::Auto {
        // "prompt": the app shows it (a `HostNotice` on `events_open`), never a
        // side channel. Re-raising the same version is a no-op.
        if let Some(notice) = update::prompt_notice(&decision) {
            tracing::info!(latest = %release.version, "update available (prompt mode)");
            if let Err(error) = update::raise_notice(paths, notice) {
                tracing::warn!(%error, "couldn't queue the update notice");
            }
        }
        return false;
    }
    let update::Check::Available { latest, file, .. } = decision else {
        return false;
    };
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    match update::download_and_replace(&file, &latest, &exe).await {
        Ok(()) => {
            tracing::info!(latest, hint = %update::restart_hint(instance_mode), "updated unipi-host");
            if instance_mode == unipi_host::instance::Mode::Manual {
                // A manual `serve` has no supervisor: say what to run next.
                println!("{}", update::restart_hint(instance_mode));
            }
            true
        }
        Err(error) => {
            tracing::warn!(%error, "automatic update failed");
            false
        }
    }
}

async fn update_command(
    paths: &HostPaths,
    check: bool,
    exe: Option<std::path::PathBuf>,
    mode: Option<String>,
    json: bool,
) -> anyhow::Result<()> {
    use unipi_host::update;
    if let Some(mode) = mode {
        let mode = if mode == "auto" {
            update::UpdateMode::Auto
        } else {
            update::UpdateMode::Prompt
        };
        update::UpdateConfig { mode }.save(&paths.config())?;
        match mode {
            update::UpdateMode::Auto => println!("Updates: automatic — `unipi-host serve` installs a newer version by itself and exits; the app (or you) starts the new binary."),
            update::UpdateMode::Prompt => println!("Updates: ask first — the UniPi app shows when a newer version is out (events_open notice); run `unipi-host update` to install it."),
        }
        return Ok(());
    }
    let current = env!("CARGO_PKG_VERSION");
    let result = update::fetch_feed().await;
    update::record(paths, now_unix(), &result);
    let release = result?;
    let decision = update::decide(&release, current, &update::current_target());
    let say = |state: &str, text: String| {
        if json {
            println!(
                "{}",
                serde_json::json!({ "state": state, "current": current, "latest": release.version, "message": text })
            );
        } else {
            println!("{text}");
        }
    };
    match decision {
        update::Check::UpToDate { .. } => say(
            "current",
            format!(
                "unipi-host {current} is up to date (latest {}).",
                release.version
            ),
        ),
        update::Check::NoBuild { latest, target } => {
            anyhow::bail!("unipi-host {latest} is out, but there is no {target} build of it");
        }
        update::Check::Available { latest, file, .. } if check => {
            let _ = file;
            say("available", format!("unipi-host {latest} is available (this is {current}). Install it: unipi-host update"));
        }
        update::Check::Available { latest, file, .. } => {
            // --exe (tests): replace that file, never the running one.
            let target = match exe {
                Some(p) => p,
                None => std::env::current_exe()?,
            };
            if !json {
                println!("Downloading unipi-host {latest}…");
            }
            update::download_and_replace(&file, &latest, &target).await?;
            // v2 has no service: this process must not keep running the old
            // image. A `serve` started by the app is respawned by the app; a
            // manual one is rerun by the owner.
            say(
                "updated",
                format!(
                    "Updated unipi-host {current} → {latest} ({}). {}",
                    target.display(),
                    update::restart_hint(unipi_host::instance::Mode::Manual)
                ),
            );
        }
    }
    Ok(())
}
