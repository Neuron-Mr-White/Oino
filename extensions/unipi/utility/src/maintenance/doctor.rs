//! `/doctor` (unipi `src/diagnostics/engine.ts`): runtime checks as a markdown report.
//!
//! | unipi plugin | Oino check |
//! |---|---|
//! | `core_directories` (`~/.unipi`, config) | the extension's state preopens are writable |
//! | `config_files` (mcp `servers.json`) | project `.oino/settings.json`, `.oino/mcp.json` parse |
//! | `node_environment` (Node version, heap) | Oino version, extension API, platform, mode |
//! | `unipi_runtime` (model cache, Decision Model key, skill exposure) | host model registry + image catalog, Decision Model key, skill exposure |
//! | `native_deps` (wreq-js, VC++ runtime) | dropped: no native bindings in WASM components |

use serde_json::Value;

/// Check status.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Status {
    /// Broken.
    Error,
    /// Degraded.
    Warning,
    /// Not determinable.
    Unknown,
    /// Fine.
    Healthy,
}

impl Status {
    /// Lower-case name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Unknown => "unknown",
            Self::Healthy => "healthy",
        }
    }
}

/// One check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    /// Id.
    pub name: String,
    /// Owning module.
    pub module: String,
    /// Status.
    pub status: Status,
    /// Message.
    pub message: String,
    /// Fix.
    pub suggestion: Option<String>,
}

impl Check {
    /// A new check.
    #[must_use]
    pub fn new(
        name: &str,
        module: &str,
        status: Status,
        message: impl Into<String>,
        suggestion: Option<&str>,
    ) -> Self {
        Self {
            name: name.into(),
            module: module.into(),
            status,
            message: message.into(),
            suggestion: suggestion.map(Into::into),
        }
    }
}

/// Overall status: the worst check (`unknown` when there are no checks).
#[must_use]
pub fn overall(checks: &[Check]) -> Status {
    let has = |s| checks.iter().any(|c| c.status == s);
    if has(Status::Error) {
        Status::Error
    } else if has(Status::Warning) {
        Status::Warning
    } else if has(Status::Healthy) {
        Status::Healthy
    } else {
        Status::Unknown
    }
}

/// Markdown report (errors first).
#[must_use]
pub fn format_report(checks: &[Check], timestamp: &str) -> String {
    let count = |s| checks.iter().filter(|c| c.status == s).count();
    let mut lines = vec![
        "## Diagnostics".to_owned(),
        String::new(),
        format!("**Overall:** {}", overall(checks).as_str().to_uppercase()),
        format!(
            "**Checks:** {} healthy, {} warning, {} error, {} unknown",
            count(Status::Healthy),
            count(Status::Warning),
            count(Status::Error),
            count(Status::Unknown)
        ),
        format!("**Timestamp:** {timestamp}"),
        String::new(),
    ];
    for status in [
        Status::Error,
        Status::Warning,
        Status::Unknown,
        Status::Healthy,
    ] {
        let group: Vec<&Check> = checks.iter().filter(|c| c.status == status).collect();
        if group.is_empty() {
            continue;
        }
        lines.push(format!(
            "### {} ({})",
            status.as_str().to_uppercase(),
            group.len()
        ));
        lines.push(String::new());
        for c in group {
            lines.push(format!("- **{}** ({})", c.name, c.module));
            lines.push(format!("  {}", c.message));
            if let Some(s) = &c.suggestion {
                lines.push(format!("  → {s}"));
            }
            lines.push(String::new());
        }
    }
    lines.join("\n")
}

/// Directory check from a write probe.
#[must_use]
pub fn dir_check(name: &str, exists: bool, writable: bool) -> Check {
    let id = format!(
        "dir_{}",
        name.trim_start_matches('/')
            .replace(|c: char| !c.is_ascii_alphanumeric(), "_")
    );
    if !exists {
        Check::new(
            &id,
            "oino-host",
            Status::Error,
            format!("Required directory missing: {name}"),
            Some("The host creates extension folders at load; restart Oino"),
        )
    } else if !writable {
        Check::new(
            &id,
            "oino-host",
            Status::Error,
            format!("Directory not writable: {name}"),
            Some("Check the permissions of ~/.oino/state and ~/.oino/{tmp,cache,logs}"),
        )
    } else {
        Check::new(
            &id,
            "oino-host",
            Status::Healthy,
            format!("Directory OK: {name}"),
            None,
        )
    }
}

/// Config file check (`None` = missing).
#[must_use]
pub fn config_check(label: &str, text: Option<&str>) -> Check {
    let id = format!("config_{}", label.rsplit('/').next().unwrap_or(label));
    match text {
        None => Check::new(
            &id,
            "oino-host",
            Status::Unknown,
            format!("Config not found: {label}"),
            None,
        ),
        Some(t) => match serde_json::from_str::<Value>(t) {
            Ok(_) => Check::new(
                &id,
                "oino-host",
                Status::Healthy,
                format!("Config valid: {label}"),
                None,
            ),
            Err(e) => Check::new(
                &id,
                "oino-host",
                Status::Error,
                format!("Invalid JSON in {label}: {e}"),
                Some("Fix or remove the config file"),
            ),
        },
    }
}

/// Model registry check.
#[must_use]
pub fn models_check(chat: usize, images: usize) -> Check {
    if chat > 0 {
        Check::new(
            "model_cache",
            "unipi.core",
            Status::Healthy,
            format!("Model registry: {chat} models ({images} image generators)"),
            None,
        )
    } else {
        Check::new(
            "model_cache",
            "unipi.core",
            Status::Warning,
            "No models with credentials — pickers and naming fall back",
            Some("Log in to a provider or add one to ~/.oino/models.json"),
        )
    }
}

/// Decision Model check.
#[must_use]
pub fn decision_check(provider: &str, model: &str, has_key: bool) -> Check {
    if has_key {
        Check::new(
            "decision_model",
            "unipi.core",
            Status::Healthy,
            format!("Decision Model: {provider}/{model}"),
            None,
        )
    } else {
        Check::new(
            "decision_model",
            "unipi.core",
            Status::Warning,
            format!(
                "Decision Model {provider}/{model} has no API key — auto-rename and skill judging fall back"
            ),
            Some("Set a key in /settings → Decision Model, or export OPENROUTER_API_KEY"),
        )
    }
}

/// Skill exposure check (`extensionSettings.skills.exposure.mode`, default `judged`).
#[must_use]
pub fn exposure_check(settings: &Value) -> Check {
    let mode = settings
        .pointer("/extensionSettings/skills/exposure/mode")
        .and_then(Value::as_str)
        .unwrap_or("judged");
    if mode == "off" {
        Check::new(
            "skill_exposure",
            "unipi.skill-registry",
            Status::Warning,
            "Skill exposure is off: UniPi's bundled skills are hidden from the model in this project",
            Some("Switch Skills → Exposure → Skill exposure to judged or all in /settings"),
        )
    } else {
        Check::new(
            "skill_exposure",
            "unipi.skill-registry",
            Status::Healthy,
            format!("Skill exposure: {mode}"),
            None,
        )
    }
}

/// Runs every check against the host.
pub async fn run() -> Vec<Check> {
    let mut checks = Vec::new();
    for dir in ["/ext-global", "/ext-workspace", "/tmp", "/cache", "/logs"] {
        let exists = std::path::Path::new(dir).is_dir();
        let probe = format!("{dir}/.doctor-probe");
        let writable = exists && std::fs::write(&probe, b"ok").is_ok();
        let _ = std::fs::remove_file(&probe);
        checks.push(dir_check(dir, exists, writable));
    }
    for (label, path) in [
        (".oino/settings.json", "/project/.oino/settings.json"),
        (".oino/mcp.json", "/project/.oino/mcp.json"),
    ] {
        checks.push(config_check(
            label,
            std::fs::read_to_string(path).ok().as_deref(),
        ));
    }
    let info = oino_extension::host::info();
    checks.push(Check::new(
        "oino_version",
        "oino-host",
        Status::Healthy,
        format!(
            "Oino {} (extension API {}), {}/{}",
            info.oino_version, info.api_version, info.platform, info.arch
        ),
        None,
    ));
    checks.push(Check::new(
        "frontends",
        "oino-host",
        Status::Healthy,
        format!(
            "Mode {:?}; frontends: {}",
            info.mode,
            if info.frontends.is_empty() {
                "none".into()
            } else {
                info.frontends.join(", ")
            }
        ),
        None,
    ));
    let chat = oino_extension::control::models().map_or(0, |m| m.len());
    let images = crate::image::models::catalog(&crate::image::models::Cache::read(), false).len();
    checks.push(models_check(chat, images));
    let all = oino_extension::control::settings().unwrap_or(Value::Null);
    let js = unipi_kit::jev::Settings::resolve(
        all.get("extensionSettings").unwrap_or(&Value::Null),
        Some(crate::settings::NAMESPACE),
    );
    let key = unipi_kit::jev::resolve_key(&js).await.is_some();
    let provider = format!("{:?}", js.provider).to_lowercase();
    checks.push(decision_check(&provider, &js.model, key));
    checks.push(exposure_check(&all));
    checks
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
    use serde_json::json;

    use super::*;

    #[test]
    fn overall_reflects_the_worst_status() {
        let ok = Check::new("a", "m", Status::Healthy, "ok", None);
        let warn = Check::new("b", "m", Status::Warning, "w", Some("fix"));
        let err = Check::new("c", "m", Status::Error, "e", None);
        assert_eq!(overall(&[ok.clone()]), Status::Healthy);
        assert_eq!(overall(&[ok.clone(), warn.clone()]), Status::Warning);
        assert_eq!(overall(&[warn.clone(), err.clone()]), Status::Error);
        assert_eq!(overall(&[]), Status::Unknown);
        let md = format_report(&[ok, warn, err], "2026-01-01T00:00:00.000Z");
        assert!(md.starts_with("## Diagnostics"));
        assert!(md.contains("**Overall:** ERROR"));
        assert!(md.contains("**Checks:** 1 healthy, 1 warning, 1 error, 0 unknown"));
        assert!(md.find("### ERROR").unwrap_or(usize::MAX) < md.find("### HEALTHY").unwrap_or(0));
        assert!(md.contains("  → fix"));
    }

    #[test]
    fn individual_checks() {
        assert_eq!(dir_check("/ext-global", true, true).status, Status::Healthy);
        assert_eq!(dir_check("/ext-global", true, true).name, "dir_ext_global");
        assert_eq!(dir_check("/tmp", false, false).status, Status::Error);
        assert!(
            dir_check("/tmp", true, false)
                .message
                .contains("not writable")
        );
        assert_eq!(config_check(".oino/mcp.json", None).status, Status::Unknown);
        assert_eq!(
            config_check(".oino/mcp.json", Some("{}")).status,
            Status::Healthy
        );
        assert_eq!(
            config_check(".oino/mcp.json", Some("{")).status,
            Status::Error
        );
        assert_eq!(models_check(0, 0).status, Status::Warning);
        assert!(
            models_check(3, 2)
                .message
                .contains("3 models (2 image generators)")
        );
        assert_eq!(
            decision_check("openrouter", "x", false).status,
            Status::Warning
        );
        assert_eq!(
            exposure_check(
                &json!({"extensionSettings": {"skills": {"exposure": {"mode": "off"}}}})
            )
            .status,
            Status::Warning
        );
        assert!(exposure_check(&json!({})).message.contains("judged"));
    }
}
