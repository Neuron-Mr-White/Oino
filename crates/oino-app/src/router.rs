use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    time::Duration,
};

use crate::{
    extension_provider_runtime::{
        format_extension_runtime_health_detail as format_generic_runtime_health_detail,
        ExtensionRuntimeHealth,
    },
    model_catalog, AppError,
};

pub(crate) const ROUTER_DEFAULT_BASE_URL: &str = "http://localhost:20128/v1";
pub(crate) const ROUTER_DEFAULT_DASHBOARD_URL: &str = "http://localhost:20128/dashboard";
pub(crate) const ROUTER_KNOWN_GOOD_TAG: &str = "3.8.7";
pub(crate) const ROUTER_IMAGE: &str = "diegosouzapw/omniroute";
pub(crate) const ROUTER_DEFAULT_CONTAINER_NAME: &str = "oino-router";
pub(crate) const ROUTER_LOCAL_DEFAULT_PASSWORD: &str = "oino";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RouterCommand {
    Help,
    Guide,
    Setup,
    Status,
    Models,
    Dashboard,
    Stop,
    Restart,
    UseExternal,
    UseManaged,
    VersionList,
    VersionPin { tag: String },
    Rollback { tag: Option<String> },
    InstallPodman,
    ResetPassword,
}

pub(crate) fn parse_router_command_input(input: &str) -> Option<RouterCommand> {
    let parts = input.split_whitespace().collect::<Vec<_>>();
    match parts.as_slice() {
        ["/router"] | ["/router", "help"] => Some(RouterCommand::Help),
        ["/router", "guide"] => Some(RouterCommand::Guide),
        ["/router", "setup"] | ["/router", "start"] => Some(RouterCommand::Setup),
        ["/router", "status"] => Some(RouterCommand::Status),
        ["/router", "models"] => Some(RouterCommand::Models),
        ["/router", "dashboard"] | ["/router", "open"] => Some(RouterCommand::Dashboard),
        ["/router", "stop"] => Some(RouterCommand::Stop),
        ["/router", "restart"] => Some(RouterCommand::Restart),
        ["/router", "use-external"] | ["/router", "external"] => Some(RouterCommand::UseExternal),
        ["/router", "use-managed"] | ["/router", "managed"] => Some(RouterCommand::UseManaged),
        ["/router", "version", "list"] | ["/router", "versions"] => {
            Some(RouterCommand::VersionList)
        }
        ["/router", "version", "pin", tag] | ["/router", "pin", tag] => {
            Some(RouterCommand::VersionPin { tag: (*tag).into() })
        }
        ["/router", "rollback"] => Some(RouterCommand::Rollback { tag: None }),
        ["/router", "rollback", tag] => Some(RouterCommand::Rollback {
            tag: Some((*tag).into()),
        }),
        ["/router", "install-podman"] => Some(RouterCommand::InstallPodman),
        ["/router", "reset-password"] | ["/router", "password", "reset"] => {
            Some(RouterCommand::ResetPassword)
        }
        _ => None,
    }
}

pub(crate) async fn execute_router_command_input(input: &str) -> Result<String, AppError> {
    let Some(command) = parse_router_command_input(input) else {
        return Err(AppError::InvalidArguments(format!(
            "unknown OmniRoute extension command `{input}`"
        )));
    };
    execute_router_command(command).await
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum RouterMode {
    #[default]
    External,
    ManagedSidecar,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct RouterConfig {
    #[serde(default)]
    pub(crate) mode: RouterMode,
    #[serde(default = "default_router_base_url")]
    pub(crate) base_url: String,
    #[serde(default = "default_router_dashboard_url")]
    pub(crate) dashboard_url: String,
    #[serde(default = "default_router_image")]
    pub(crate) image: String,
    #[serde(default = "default_router_known_good_tag")]
    pub(crate) known_good_tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) pinned_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_good_tag: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) last_attempted_tag: Option<String>,
    #[serde(default)]
    pub(crate) allow_latest_probe: bool,
    #[serde(default = "default_router_healthcheck_timeout_ms")]
    pub(crate) healthcheck_timeout_ms: u64,
    #[serde(default = "default_router_container_name")]
    pub(crate) container_name: String,
    #[serde(default = "default_router_host_port")]
    pub(crate) host_port: u16,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) host_data_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) initial_password: Option<String>,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            mode: RouterMode::External,
            base_url: default_router_base_url(),
            dashboard_url: default_router_dashboard_url(),
            image: default_router_image(),
            known_good_tag: default_router_known_good_tag(),
            pinned_tag: None,
            last_good_tag: Some(default_router_known_good_tag()),
            last_attempted_tag: None,
            allow_latest_probe: false,
            healthcheck_timeout_ms: default_router_healthcheck_timeout_ms(),
            container_name: default_router_container_name(),
            host_port: default_router_host_port(),
            host_data_dir: None,
            initial_password: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RouterHealth {
    pub(crate) reachable: bool,
    pub(crate) status: Option<String>,
    pub(crate) model_count: Option<usize>,
    pub(crate) error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ContainerRuntimeStatus {
    pub(crate) name: &'static str,
    pub(crate) available: bool,
    pub(crate) detail: String,
}

pub(crate) fn default_router_base_url() -> String {
    ROUTER_DEFAULT_BASE_URL.into()
}

pub(crate) fn default_router_dashboard_url() -> String {
    ROUTER_DEFAULT_DASHBOARD_URL.into()
}

pub(crate) fn default_router_image() -> String {
    ROUTER_IMAGE.into()
}

pub(crate) fn default_router_known_good_tag() -> String {
    ROUTER_KNOWN_GOOD_TAG.into()
}

const fn default_router_healthcheck_timeout_ms() -> u64 {
    10_000
}

pub(crate) fn default_router_container_name() -> String {
    ROUTER_DEFAULT_CONTAINER_NAME.into()
}

const fn default_router_host_port() -> u16 {
    20128
}

pub(crate) fn router_config_path() -> Result<PathBuf, AppError> {
    let Some(home) = dirs::home_dir() else {
        return Err(AppError::InvalidArguments(
            "home directory unavailable for OmniRoute config".into(),
        ));
    };
    Ok(home.join(".oino/extensions/router/config.json"))
}

pub(crate) fn load_router_config() -> Result<RouterConfig, AppError> {
    let path = router_config_path()?;
    if !path.exists() {
        return Ok(RouterConfig::default());
    }
    let text = fs::read_to_string(&path)?;
    serde_json::from_str(&text).map_err(|err| {
        AppError::InvalidArguments(format!(
            "invalid OmniRoute config at {}: {err}",
            path.display()
        ))
    })
}

pub(crate) fn save_router_config(config: &RouterConfig) -> Result<PathBuf, AppError> {
    let path = router_config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|err| {
        AppError::InvalidArguments(format!("could not serialize OmniRoute config: {err}"))
    })?;
    fs::write(&path, format!("{text}\n"))?;
    Ok(path)
}

pub(crate) fn resolved_router_base_url(config: &RouterConfig) -> String {
    std::env::var("OMNIROUTE_BASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| config.base_url.clone())
}

pub(crate) fn resolved_router_dashboard_url(config: &RouterConfig) -> String {
    std::env::var("OMNIROUTE_DASHBOARD_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| config.dashboard_url.clone())
}

pub(crate) fn resolved_router_tag(config: &RouterConfig) -> String {
    std::env::var("OMNIROUTE_IMAGE_TAG")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| config.pinned_tag.clone())
        .or_else(|| config.last_good_tag.clone())
        .unwrap_or_else(|| config.known_good_tag.clone())
}

pub(crate) fn format_extension_runtime_health_detail(health: &ExtensionRuntimeHealth) -> String {
    let health_detail = format_generic_runtime_health_detail(health);
    match load_router_config() {
        Ok(config) => format!(
            "{} {}",
            format_extension_config_detail(&config),
            health_detail
        ),
        Err(err) => format!("OmniRoute config could not be loaded: {err}. {health_detail}"),
    }
}

pub(crate) fn format_extension_config_detail(config: &RouterConfig) -> String {
    let config_path = router_config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "~/.oino/extensions/router/config.json".into());
    format_extension_config_detail_with_path(config, &config_path)
}

pub(crate) fn format_extension_config_detail_with_path(
    config: &RouterConfig,
    config_path: &str,
) -> String {
    let base_url = resolved_router_base_url(config);
    let dashboard_url = resolved_router_dashboard_url(config);
    let tag = resolved_router_tag(config);
    format!(
        "OmniRoute config: mode {:?}, endpoint {}, dashboard {}, image {}:{}, pinned {}, last-good {}, config {}.",
        config.mode,
        base_url,
        dashboard_url,
        config.image,
        tag,
        config.pinned_tag.as_deref().unwrap_or("none"),
        config.last_good_tag.as_deref().unwrap_or("none"),
        config_path,
    )
}

#[cfg(test)]
pub(crate) fn format_extension_readiness_detail(
    config: &RouterConfig,
    config_path: &str,
    health: &RouterHealth,
) -> String {
    let health = ExtensionRuntimeHealth {
        url: format!(
            "{}/models",
            resolved_router_base_url(config).trim_end_matches('/')
        ),
        reachable: health.reachable,
        status: health.status.clone(),
        model_count: health.model_count,
        error: health.error.clone(),
    };
    format!(
        "{} {}",
        format_extension_config_detail_with_path(config, config_path),
        format_generic_runtime_health_detail(&health).replace("Live runtime health", "Live health")
    )
}

pub(crate) async fn execute_router_command(command: RouterCommand) -> Result<String, AppError> {
    match command {
        RouterCommand::Help => Ok(format_router_help()),
        RouterCommand::Guide => Ok(format_router_guide()),
        RouterCommand::Setup => setup_router().await,
        RouterCommand::Status => router_status().await,
        RouterCommand::Models => router_models().await,
        RouterCommand::Dashboard => {
            let config = load_router_config()?;
            let url = resolved_router_dashboard_url(&config);
            match webbrowser::open(&url) {
                Ok(()) => Ok(format!("Opened OmniRoute dashboard: {url}")),
                Err(err) => Ok(format!(
                    "Could not open browser ({err}). Open OmniRoute dashboard manually: {url}"
                )),
            }
        }
        RouterCommand::Stop => stop_router_sidecar(),
        RouterCommand::Restart => restart_router_sidecar().await,
        RouterCommand::UseExternal => set_router_mode(RouterMode::External),
        RouterCommand::UseManaged => set_router_mode(RouterMode::ManagedSidecar),
        RouterCommand::VersionList => router_version_list().await,
        RouterCommand::VersionPin { tag } => pin_router_tag(&tag),
        RouterCommand::Rollback { tag } => rollback_router_tag(tag.as_deref()),
        RouterCommand::InstallPodman => install_podman().await,
        RouterCommand::ResetPassword => reset_router_password().await,
    }
}

pub(crate) fn format_router_help() -> String {
    [
        "OmniRoute commands:",
        "  /router setup              Initialize and start managed OmniRoute sidecar",
        "  /router guide              Show setup guide without changing anything",
        "  /router status             Check endpoint health",
        "  /router models             Fetch model catalog from /v1/models",
        "  /router dashboard          Open dashboard",
        "  /router stop               Stop managed sidecar",
        "  /router restart            Restart managed sidecar with fallback",
        "  /router use-external       Use external endpoint mode",
        "  /router use-managed        Use managed sidecar mode",
        "  /router version list       List published container tags",
        "  /router version pin <tag>  Pin requested tag (config wiring next)",
        "  /router rollback [tag]     Roll back to known-good/requested tag",
        "  /router install-podman     Best-effort Podman install helper",
        "  /router reset-password     Reset dashboard password to Oino's initial password",
    ]
    .join("\n")
}

pub(crate) fn format_router_guide() -> String {
    let config = load_router_config().unwrap_or_default();
    let tag = resolved_router_tag(&config);
    let run_command = router_managed_run_command(&config, &tag);
    let config_path = router_config_path()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "~/.oino/extensions/router/config.json".into());
    format!(
        "OmniRoute guide\n\nRecommended flow:\n1. Run `/router setup` to initialize config and start the managed sidecar.\n2. Open `/router dashboard`.\n3. Login with the initial dashboard password shown by setup/start. If you already changed the password in OmniRoute, use your saved password instead.\n4. Add provider accounts/API keys in OmniRoute.\n5. Run `/router models`.\n6. Select `/model router:kr/claude-sonnet-4.5` or another returned model.\n\nExternal endpoint mode:\n1. Start or connect to OmniRoute at {base}.\n2. Run `/router use-external`.\n3. Open dashboard: {dashboard}\n4. If REQUIRE_API_KEY=true, set OMNIROUTE_API_KEY for Oino.\n\nManaged sidecar command:\n  {run_command}\n\nConfig: {config_path}\nResolved image: {image}:{tag}\nFallback order: pinned tag -> last-good tag -> known-good tag ({known_good}).",
        base = resolved_router_base_url(&config),
        dashboard = resolved_router_dashboard_url(&config),
        image = config.image,
        known_good = config.known_good_tag,
    )
}

pub(crate) async fn setup_router() -> Result<String, AppError> {
    let config_path = router_config_path()?;
    let was_existing = config_path.exists();
    let mut config = load_router_config()?;
    let runtimes = detect_container_runtimes();
    let preferred_runtime = runtimes.iter().find(|runtime| runtime.available);

    ensure_router_initial_password(&mut config);
    config.mode = RouterMode::ManagedSidecar;
    config.base_url = format!("http://localhost:{}/v1", config.host_port);
    config.dashboard_url = format!("http://localhost:{}/dashboard", config.host_port);

    let data_dir = router_managed_data_dir(&config);
    fs::create_dir_all(&data_dir)?;
    let saved_path = save_router_config(&config)?;
    let tag = resolved_router_tag(&config);
    let runtime_summary = runtimes
        .iter()
        .map(|runtime| format!("{}: {}", runtime.name, runtime.detail))
        .collect::<Vec<_>>()
        .join("; ");
    let init_summary = format!(
        "OmniRoute setup initialized.\nConfig: {}{}\nManaged data dir: {data_dir}\nMode: {:?}\nEndpoint: {}\nDashboard: {}\nInitial dashboard password: {}\nResolved image: {}:{tag}\nContainer runtimes: {runtime_summary}",
        saved_path.display(),
        if was_existing { " (updated)" } else { " (created)" },
        config.mode,
        resolved_router_base_url(&config),
        resolved_router_dashboard_url(&config),
        router_initial_password(&config),
        config.image,
    );

    let health = check_router_health(&config).await;
    if health.reachable {
        return Ok(format!(
            "{init_summary}\n\nOmniRoute is already reachable{}\nNext: `/router dashboard`, then `/router models`.",
            health
                .model_count
                .map(|count| format!(" · {count} models"))
                .unwrap_or_default()
        ));
    }

    if preferred_runtime.is_none() {
        return Ok(format!(
            "{init_summary}\n\nNo Docker/Podman runtime was found.\nPrompt: run `/router install-podman` to let Oino try a best-effort Podman install, or install Docker/Podman yourself.\nAfter that, run `/router setup` again.\n\nNo provider API keys are stored in Oino; add them in the OmniRoute dashboard after startup."
        ));
    }

    let start = start_router_sidecar().await?;
    Ok(format!(
        "{init_summary}\n\n{start}\n\nNext: open `/router dashboard`, login with the password above if this is a fresh OmniRoute data dir, add provider keys, then run `/router models`.\nIf login fails, this data dir probably already has a saved OmniRoute password hash; run `/router reset-password`, then `/router restart`."
    ))
}

pub(crate) async fn router_status() -> Result<String, AppError> {
    let config = load_router_config()?;
    let base_url = resolved_router_base_url(&config);
    let dashboard_url = resolved_router_dashboard_url(&config);
    let models_url = format!("{}/models", base_url.trim_end_matches('/'));
    let health = check_router_health(&config).await;
    let runtimes = detect_container_runtimes();
    let runtime_summary = runtimes
        .iter()
        .map(|runtime| format!("{}: {}", runtime.name, runtime.detail))
        .collect::<Vec<_>>()
        .join("; ");
    let config_path = router_config_path()?.display().to_string();
    let tag = resolved_router_tag(&config);
    let health_line = if health.reachable {
        format!(
            "reachable{}",
            health
                .model_count
                .map(|count| format!(" · {count} models"))
                .unwrap_or_default()
        )
    } else {
        format!(
            "not reachable{}",
            health
                .error
                .as_deref()
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        )
    };
    Ok(format!(
        "OmniRoute status: {health_line}\nEndpoint: {base_url}\nModels: {models_url}\nDashboard: {dashboard_url}\nConfig: {config_path}\nMode: {:?}\nResolved image: {}:{tag}\nPinned: {}\nLast good: {}\nContainer runtimes: {runtime_summary}",
        config.mode,
        config.image,
        config.pinned_tag.as_deref().unwrap_or("none"),
        config.last_good_tag.as_deref().unwrap_or("none"),
    ))
}

pub(crate) fn pin_router_tag(tag: &str) -> Result<String, AppError> {
    let tag = validate_router_tag(tag)?;
    let mut config = load_router_config()?;
    config.pinned_tag = Some(tag.clone());
    config.last_attempted_tag = Some(tag.clone());
    let path = save_router_config(&config)?;
    Ok(format!(
        "Pinned OmniRoute image tag to `{tag}`.\nConfig: {}\nResolved image: {}:{tag}\nRun `/router status` to health-check the active endpoint. Managed sidecar restart wiring lands next.",
        path.display(),
        config.image
    ))
}

pub(crate) fn rollback_router_tag(tag: Option<&str>) -> Result<String, AppError> {
    let mut config = load_router_config()?;
    let target = tag
        .map(validate_router_tag)
        .transpose()?
        .or_else(|| config.last_good_tag.clone())
        .unwrap_or_else(|| config.known_good_tag.clone());
    config.pinned_tag = Some(target.clone());
    config.last_attempted_tag = Some(target.clone());
    let path = save_router_config(&config)?;
    Ok(format!(
        "Prepared OmniRoute rollback to `{target}`.\nConfig: {}\nResolved image: {}:{target}\nNext managed-sidecar iteration will restart and health-check this tag automatically.",
        path.display(),
        config.image
    ))
}

pub(crate) fn validate_router_tag(tag: &str) -> Result<String, AppError> {
    let tag = tag.trim();
    if tag.is_empty()
        || tag.len() > 128
        || !tag
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
    {
        return Err(AppError::InvalidArguments(format!(
            "invalid OmniRoute image tag `{tag}`"
        )));
    }
    Ok(tag.into())
}

pub(crate) fn router_managed_data_dir(config: &RouterConfig) -> String {
    config.host_data_dir.clone().unwrap_or_else(|| {
        dirs::home_dir()
            .map(|home| home.join(".oino/extensions/router/data"))
            .unwrap_or_else(|| PathBuf::from("$HOME/.oino/extensions/router/data"))
            .display()
            .to_string()
    })
}

pub(crate) fn router_sqlite_path(config: &RouterConfig) -> PathBuf {
    PathBuf::from(router_managed_data_dir(config))
        .join("db")
        .join("data.sqlite")
}

pub(crate) fn ensure_router_initial_password(config: &mut RouterConfig) -> String {
    if config
        .initial_password
        .as_deref()
        .is_none_or(|password| password.trim().is_empty() || password.starts_with("oino-"))
    {
        config.initial_password = Some(ROUTER_LOCAL_DEFAULT_PASSWORD.into());
    }
    config
        .initial_password
        .clone()
        .unwrap_or_else(|| ROUTER_LOCAL_DEFAULT_PASSWORD.into())
}

pub(crate) fn router_initial_password(config: &RouterConfig) -> String {
    config
        .initial_password
        .clone()
        .unwrap_or_else(|| ROUTER_LOCAL_DEFAULT_PASSWORD.into())
}

pub(crate) fn router_managed_run_command(config: &RouterConfig, tag: &str) -> String {
    format!(
        "docker run -d --name {name} -p {port}:20128 -v \"{data}:/app/data\" -e DATA_DIR=/app/data -e PORT=20128 -e HOSTNAME=0.0.0.0 -e INITIAL_PASSWORD={password} {image}:{tag}",
        name = config.container_name,
        port = config.host_port,
        data = router_managed_data_dir(config),
        password = router_initial_password(config),
        image = config.image,
    )
}

pub(crate) fn detect_container_runtimes() -> Vec<ContainerRuntimeStatus> {
    ["docker", "podman"]
        .into_iter()
        .map(|name| {
            let output = Command::new(name).arg("--version").output();
            match output {
                Ok(output) if output.status.success() => ContainerRuntimeStatus {
                    name,
                    available: true,
                    detail: String::from_utf8_lossy(&output.stdout).trim().to_string(),
                },
                Ok(output) => ContainerRuntimeStatus {
                    name,
                    available: false,
                    detail: format!("unavailable (exit {})", output.status),
                },
                Err(err) => ContainerRuntimeStatus {
                    name,
                    available: false,
                    detail: err.to_string(),
                },
            }
        })
        .collect()
}

pub(crate) async fn check_router_health(config: &RouterConfig) -> RouterHealth {
    let base_url = resolved_router_base_url(config);
    let models_url = format!("{}/models", base_url.trim_end_matches('/'));
    let timeout = Duration::from_millis(config.healthcheck_timeout_ms.clamp(1_000, 60_000));
    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(client) => client,
        Err(err) => {
            return RouterHealth {
                reachable: false,
                status: None,
                model_count: None,
                error: Some(err.to_string()),
            }
        }
    };
    let mut request = client.get(&models_url);
    if let Ok(api_key) = std::env::var("OMNIROUTE_API_KEY") {
        if !api_key.trim().is_empty() {
            request = request.bearer_auth(api_key);
        }
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            return RouterHealth {
                reachable: false,
                status: None,
                model_count: None,
                error: Some(err.to_string()),
            }
        }
    };
    let status = response.status().to_string();
    if !response.status().is_success() {
        return RouterHealth {
            reachable: false,
            status: Some(status.clone()),
            model_count: None,
            error: Some(format!("/v1/models returned {status}")),
        };
    }
    let model_count = response
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|body| router_model_count(&body));
    RouterHealth {
        reachable: true,
        status: Some(status),
        model_count,
        error: None,
    }
}

pub(crate) fn router_model_count(body: &serde_json::Value) -> Option<usize> {
    body.get("data")
        .and_then(|value| value.as_array())
        .or_else(|| body.get("models").and_then(|value| value.as_array()))
        .map(Vec::len)
}

pub(crate) fn set_router_mode(mode: RouterMode) -> Result<String, AppError> {
    let mut config = load_router_config()?;
    config.mode = mode;
    if mode == RouterMode::ManagedSidecar {
        config.base_url = format!("http://localhost:{}/v1", config.host_port);
        config.dashboard_url = format!("http://localhost:{}/dashboard", config.host_port);
    }
    let path = save_router_config(&config)?;
    Ok(format!(
        "OmniRoute mode set to {:?}.\nConfig: {}\nEndpoint: {}",
        config.mode,
        path.display(),
        config.base_url
    ))
}

pub(crate) fn preferred_container_runtime() -> Result<ContainerRuntimeStatus, AppError> {
    detect_container_runtimes()
        .into_iter()
        .find(|runtime| runtime.available)
        .ok_or_else(|| {
            AppError::InvalidArguments(
                "No container runtime found. Run `/router install-podman`, install Docker/Podman yourself, or use /router use-external."
                    .into(),
            )
        })
}

pub(crate) fn command_available(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
        || Command::new("which")
            .arg(name)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
}

pub(crate) fn current_user_is_root() -> bool {
    Command::new("id")
        .arg("-u")
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .is_some_and(|uid| uid.trim() == "0")
}

pub(crate) fn passwordless_sudo_available() -> bool {
    Command::new("sudo")
        .arg("-n")
        .arg("true")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

pub(crate) fn shell_args(script: &str) -> Vec<String> {
    vec!["-lc".into(), script.into()]
}

pub(crate) async fn reset_router_password() -> Result<String, AppError> {
    let mut config = load_router_config()?;
    let password = ensure_router_initial_password(&mut config);
    let config_path = save_router_config(&config)?;
    let db_path = router_sqlite_path(&config);
    if !db_path.exists() {
        return Ok(format!(
            "OmniRoute database was not found at {}.\nConfig saved: {}\nInitial dashboard password: {password}\n\nRun `/router setup` to start OmniRoute with this password.",
            db_path.display(),
            config_path.display()
        ));
    }

    let script = r#"
import json, sqlite3, sys
path = sys.argv[1]
conn = sqlite3.connect(path)
try:
    row = conn.execute('SELECT data FROM settings WHERE id = 1').fetchone()
    data = json.loads(row[0]) if row and row[0] else {}
    data.pop('password', None)
    data['authMode'] = 'password'
    data['requireLogin'] = True
    payload = json.dumps(data, separators=(',', ':'))
    conn.execute('INSERT INTO settings(id, data) VALUES(1, ?) ON CONFLICT(id) DO UPDATE SET data = excluded.data', (payload,))
    conn.commit()
finally:
    conn.close()
"#;
    let output = command_output_with_timeout(
        "python3",
        &["-c".into(), script.into(), db_path.display().to_string()],
        Duration::from_secs(20),
    )?;
    if !output.status.success() {
        return Err(AppError::InvalidArguments(format!(
            "Could not reset OmniRoute dashboard password{}",
            command_output_detail(&output)
        )));
    }

    Ok(format!(
        "Reset OmniRoute dashboard password state.\nConfig: {}\nDatabase: {}\nInitial dashboard password: {password}\n\nIf the dashboard still rejects it, run `/router restart` so the container picks up INITIAL_PASSWORD again, then login with the password above.",
        config_path.display(),
        db_path.display()
    ))
}

pub(crate) async fn install_podman() -> Result<String, AppError> {
    if command_available("podman") {
        return Ok("Podman is already installed. Run `/router setup` to start OmniRoute.".into());
    }

    let os = std::env::consts::OS;
    let mut notes = Vec::new();
    let script = if os == "macos" && command_available("brew") {
        "brew install podman && (podman machine init || true) && podman machine start".to_string()
    } else if os == "linux" {
        let prefix = if current_user_is_root() {
            ""
        } else if passwordless_sudo_available() {
            "sudo -n "
        } else {
            return Ok(
                "No Docker/Podman runtime found, and Oino cannot install Podman non-interactively because this Linux user is not root and passwordless sudo is unavailable.\n\nInstall Podman with your distro package manager, then run `/router setup` again. Examples:\n  Ubuntu/Debian: sudo apt-get update && sudo apt-get install -y podman\n  Fedora: sudo dnf install -y podman\n  Arch: sudo pacman -Sy --noconfirm podman\n\nPodman itself can run rootless after installation.".into(),
            );
        };
        if command_available("apt-get") {
            format!("DEBIAN_FRONTEND=noninteractive {prefix}apt-get update && DEBIAN_FRONTEND=noninteractive {prefix}apt-get install -y podman")
        } else if command_available("dnf") {
            format!("{prefix}dnf install -y podman")
        } else if command_available("yum") {
            format!("{prefix}yum install -y podman")
        } else if command_available("pacman") {
            format!("{prefix}pacman -Sy --noconfirm podman")
        } else if command_available("zypper") {
            format!("{prefix}zypper --non-interactive install podman")
        } else if command_available("apk") {
            format!("{prefix}apk add podman")
        } else {
            return Ok(
                "No supported package manager found for automatic Podman installation. Install Podman manually, then run `/router setup` again.".into(),
            );
        }
    } else {
        return Ok(format!(
            "Automatic Podman installation is not supported on {os}. Install Podman manually, then run `/router setup` again."
        ));
    };

    notes.push(format!("Running Podman install helper:\n  {script}"));
    let output = command_output_with_timeout("sh", &shell_args(&script), Duration::from_secs(600))?;
    if !output.status.success() {
        return Err(AppError::InvalidArguments(format!(
            "Podman install helper failed{}",
            command_output_detail(&output)
        )));
    }
    let detail = command_output_detail(&output);
    if !detail.is_empty() {
        notes.push(format!("Installer output{detail}"));
    }
    if command_available("podman") {
        notes.push("Podman is now available. Run `/router setup` to start OmniRoute.".into());
    } else {
        notes.push("Installer completed, but `podman --version` is still unavailable in this shell. Restart your terminal or install Podman manually, then run `/router setup` again.".into());
    }
    Ok(notes.join("\n\n"))
}

pub(crate) fn command_output_with_timeout(
    program: &str,
    args: &[String],
    timeout: Duration,
) -> Result<std::process::Output, AppError> {
    let mut child = Command::new(program)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| AppError::InvalidArguments(format!("{program} failed to start: {err}")))?;
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => {
                return child.wait_with_output().map_err(|err| {
                    AppError::InvalidArguments(format!("{program} output collection failed: {err}"))
                });
            }
            Ok(None) if start.elapsed() >= timeout => {
                let _ = child.kill();
                let output = child.wait_with_output().map_err(|err| {
                    AppError::InvalidArguments(format!("{program} timeout cleanup failed: {err}"))
                })?;
                return Err(AppError::InvalidArguments(format!(
                    "{program} {} timed out after {}s{}",
                    args.join(" "),
                    timeout.as_secs(),
                    command_output_detail(&output)
                )));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(err) => {
                let _ = child.kill();
                return Err(AppError::InvalidArguments(format!(
                    "{program} status check failed: {err}"
                )));
            }
        }
    }
}

pub(crate) fn command_output_detail(output: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let detail = if stderr.is_empty() { stdout } else { stderr };
    if detail.is_empty() {
        String::new()
    } else {
        format!(": {detail}")
    }
}

pub(crate) fn run_container_runtime_command(
    runtime: &ContainerRuntimeStatus,
    args: &[String],
) -> Result<String, AppError> {
    run_container_runtime_command_with_timeout(runtime, args, Duration::from_secs(120))
}

pub(crate) fn run_container_runtime_command_with_timeout(
    runtime: &ContainerRuntimeStatus,
    args: &[String],
    timeout: Duration,
) -> Result<String, AppError> {
    let output = command_output_with_timeout(runtime.name, args, timeout)?;
    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(stdout)
    } else {
        Err(AppError::InvalidArguments(format!(
            "{} {} failed{}",
            runtime.name,
            args.join(" "),
            command_output_detail(&output)
        )))
    }
}

pub(crate) fn router_start_candidates(config: &RouterConfig) -> Vec<String> {
    let mut candidates = Vec::new();
    candidates.push(resolved_router_tag(config));
    if let Some(tag) = &config.last_good_tag {
        candidates.push(tag.clone());
    }
    candidates.push(config.known_good_tag.clone());
    candidates.retain(|tag| !tag.trim().is_empty());
    candidates.dedup();
    candidates
}

pub(crate) fn router_run_args(config: &RouterConfig, tag: &str) -> Vec<String> {
    vec![
        "run".into(),
        "-d".into(),
        "--name".into(),
        config.container_name.clone(),
        "-p".into(),
        format!("{}:20128", config.host_port),
        "-v".into(),
        format!("{}:/app/data", router_managed_data_dir(config)),
        "-e".into(),
        "DATA_DIR=/app/data".into(),
        "-e".into(),
        "PORT=20128".into(),
        "-e".into(),
        "HOSTNAME=0.0.0.0".into(),
        "-e".into(),
        format!("INITIAL_PASSWORD={}", router_initial_password(config)),
        format!("{}:{tag}", config.image),
    ]
}

pub(crate) fn router_recent_logs(
    runtime: &ContainerRuntimeStatus,
    config: &RouterConfig,
) -> String {
    run_container_runtime_command_with_timeout(
        runtime,
        &[
            "logs".into(),
            "--tail".into(),
            "120".into(),
            config.container_name.clone(),
        ],
        Duration::from_secs(10),
    )
    .unwrap_or_else(|err| format!("Could not read container logs: {err}"))
}

pub(crate) fn router_startup_secret_lines(logs: &str) -> Vec<String> {
    logs.lines()
        .filter(|line| {
            let lower = line.to_ascii_lowercase();
            lower.contains("password")
                || lower.contains("api key")
                || lower.contains("apikey")
                || lower.contains("initial_password")
                || lower.contains("initial password")
        })
        .take(12)
        .map(|line| line.trim().to_string())
        .collect()
}

pub(crate) fn format_router_startup_credentials(config: &RouterConfig, logs: &str) -> String {
    let mut lines = vec![format!(
        "Dashboard login: {} (use this only for a fresh OmniRoute data dir; if you changed it in OmniRoute, use your saved password)",
        router_initial_password(config)
    )];
    let secret_lines = router_startup_secret_lines(logs);
    if !secret_lines.is_empty() {
        lines.push("Relevant startup log lines:".into());
        lines.extend(secret_lines.into_iter().map(|line| format!("  {line}")));
    }
    lines.join("\n")
}

pub(crate) fn stop_router_sidecar() -> Result<String, AppError> {
    let config = load_router_config()?;
    let runtime = preferred_container_runtime()?;
    match run_container_runtime_command(
        &runtime,
        &["rm".into(), "-f".into(), config.container_name.clone()],
    ) {
        Ok(output) => Ok(format!(
            "Stopped OmniRoute sidecar `{}` with {}.{}",
            config.container_name,
            runtime.name,
            if output.is_empty() {
                String::new()
            } else {
                format!("\n{output}")
            }
        )),
        Err(err) => Err(err),
    }
}

pub(crate) async fn start_router_sidecar() -> Result<String, AppError> {
    let mut config = load_router_config()?;
    config.mode = RouterMode::ManagedSidecar;
    config.base_url = format!("http://localhost:{}/v1", config.host_port);
    config.dashboard_url = format!("http://localhost:{}/dashboard", config.host_port);
    ensure_router_initial_password(&mut config);
    let runtime = preferred_container_runtime()?;
    let data_dir = router_managed_data_dir(&config);
    fs::create_dir_all(&data_dir)?;

    let candidates = router_start_candidates(&config);
    let mut attempts = Vec::new();
    for tag in candidates {
        config.last_attempted_tag = Some(tag.clone());
        let _ = run_container_runtime_command_with_timeout(
            &runtime,
            &["rm".into(), "-f".into(), config.container_name.clone()],
            Duration::from_secs(15),
        );
        let image = format!("{}:{tag}", config.image);
        let pull = run_container_runtime_command_with_timeout(
            &runtime,
            &["pull".into(), image.clone()],
            Duration::from_secs(180),
        );
        if let Err(err) = &pull {
            attempts.push(format!("{tag}: pull failed ({err})"));
        }
        match run_container_runtime_command_with_timeout(
            &runtime,
            &router_run_args(&config, &tag),
            Duration::from_secs(30),
        ) {
            Ok(container_id) => {
                let health = wait_for_router_health(&config).await;
                if health.reachable {
                    config.last_good_tag = Some(tag.clone());
                    config.pinned_tag = Some(tag.clone());
                    let path = save_router_config(&config)?;
                    let logs = router_recent_logs(&runtime, &config);
                    let credentials = format_router_startup_credentials(&config, &logs);
                    return Ok(format!(
                        "Started OmniRoute sidecar with {} image {image}.\nContainer: {}\nHealth: reachable{}\nConfig: {}\n\n{credentials}",
                        runtime.name,
                        if container_id.is_empty() { config.container_name.clone() } else { container_id },
                        health
                            .model_count
                            .map(|count| format!(" · {count} models"))
                            .unwrap_or_default(),
                        path.display()
                    ));
                }
                attempts.push(format!(
                    "{tag}: started but health failed ({})",
                    health
                        .error
                        .unwrap_or_else(|| "unknown health error".into())
                ));
            }
            Err(err) => attempts.push(format!("{tag}: start failed ({err})")),
        }
    }
    let _ = save_router_config(&config);
    Err(AppError::InvalidArguments(format!(
        "Could not start a healthy OmniRoute sidecar. Attempts:\n  {}",
        attempts.join("\n  ")
    )))
}

pub(crate) async fn restart_router_sidecar() -> Result<String, AppError> {
    let stop = stop_router_sidecar().unwrap_or_else(|err| format!("Stop warning: {err}"));
    let start = start_router_sidecar().await?;
    Ok(format!("{stop}\n\n{start}"))
}

pub(crate) async fn wait_for_router_health(config: &RouterConfig) -> RouterHealth {
    let deadline = std::time::Instant::now()
        + Duration::from_millis(config.healthcheck_timeout_ms.clamp(1_000, 60_000));
    let mut last = check_router_health(config).await;
    while !last.reachable && std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(500)).await;
        last = check_router_health(config).await;
    }
    last
}

pub(crate) async fn router_models() -> Result<String, AppError> {
    let config = load_router_config()?;
    let base_url = resolved_router_base_url(&config);
    let pricing_status = match model_catalog::refresh_openrouter_pricing_sidecar(
        &oino_provider_openrouter::OpenRouterConfig::default(),
    )
    .await
    {
        Ok(count) => format!("OpenRouter pricing sidecar: {count} models"),
        Err(err) => format!("OpenRouter pricing sidecar unavailable: {err}"),
    };
    let update = model_catalog::refresh_openai_proxy_update(
        model_catalog::ROUTER_PROVIDER_ID,
        "router",
        &base_url,
        Some("OMNIROUTE_API_KEY"),
    )
    .await;
    let preview = update
        .models
        .iter()
        .filter(|model| model.provider == model_catalog::ROUTER_PROVIDER_ID)
        .take(12)
        .map(|model| format!("  {}", model.id))
        .collect::<Vec<_>>()
        .join(
            "
",
        );
    Ok(if preview.is_empty() {
        format!("{}; {pricing_status}", update.status)
    } else {
        format!(
            "{}; {}

{preview}",
            update.status, pricing_status
        )
    })
}

pub(crate) async fn router_version_list() -> Result<String, AppError> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|err| AppError::InvalidArguments(err.to_string()))?;
    let tags: serde_json::Value = client
        .get("https://registry.hub.docker.com/v2/repositories/diegosouzapw/omniroute/tags?page_size=100")
        .send()
        .await
        .map_err(|err| AppError::InvalidArguments(format!("Docker Hub tag request failed: {err}")))?
        .json()
        .await
        .map_err(|err| AppError::InvalidArguments(format!("invalid Docker Hub tag JSON: {err}")))?;
    let mut tags = tags
        .get("results")
        .and_then(|tags| tags.as_array())
        .into_iter()
        .flatten()
        .filter_map(|tag| tag.get("name").and_then(|name| name.as_str()))
        .filter(|tag| tag.chars().next().is_some_and(|ch| ch.is_ascii_digit()))
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    tags.sort_by(|left, right| semverish_tag_key(right).cmp(&semverish_tag_key(left)));
    tags.dedup();
    let preview = tags.into_iter().take(20).collect::<Vec<_>>().join("\n  ");
    Ok(format!(
        "OmniRoute published semver tags (newest first; known-good {ROUTER_KNOWN_GOOD_TAG}):\n  {preview}"
    ))
}

pub(crate) fn semverish_tag_key(tag: &str) -> (u64, u64, u64, String) {
    let mut parts = tag.split('.');
    let major = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let minor = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    let patch = parts
        .next()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0);
    (major, minor, patch, tag.into())
}
