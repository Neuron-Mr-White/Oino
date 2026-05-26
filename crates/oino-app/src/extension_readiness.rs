use std::collections::BTreeSet;

use oino_extension_core::{
    ActiveContribution, AuthContribution, AuthFlowType, InactiveContribution, Provenance,
    ProviderContribution, ProviderRuntimeModelIdPolicy, ProviderRuntimeProtocol,
    ProviderRuntimeSecret,
};
use oino_extension_manager::ExtensionManagerSnapshot;
use oino_tui::AuthStatusItem;

use crate::extension_provider_runtime::{check_extension_runtime_health, ExtensionRuntimeHealth};

pub(crate) fn provider_matches(snapshot: &ExtensionManagerSnapshot, provider: &str) -> bool {
    snapshot
        .registries
        .auth_providers
        .active
        .iter()
        .any(|active| active.entry.contribution.provider_id == provider)
        || snapshot
            .registries
            .auth_providers
            .inactive
            .iter()
            .any(|inactive| inactive.entry.contribution.provider_id == provider)
        || snapshot
            .registries
            .providers
            .active
            .iter()
            .any(|active| active.entry.contribution.provider_id == provider)
        || snapshot
            .registries
            .providers
            .inactive
            .iter()
            .any(|inactive| inactive.entry.contribution.provider_id == provider)
}

pub(crate) async fn status_items_with_health(
    snapshot: &ExtensionManagerSnapshot,
    provider_filter: Option<&str>,
    current_provider: &str,
    runtime_detail: &(dyn Fn(&str, &ExtensionRuntimeHealth) -> String + Sync),
) -> Vec<AuthStatusItem> {
    let mut rows = status_items(snapshot, provider_filter, current_provider);
    enrich_runtime_status_rows(&mut rows, snapshot, provider_filter, runtime_detail).await;
    rows
}

pub(crate) fn status_items(
    snapshot: &ExtensionManagerSnapshot,
    provider_filter: Option<&str>,
    current_provider: &str,
) -> Vec<AuthStatusItem> {
    let mut rows = Vec::new();
    let mut auth_provider_ids = BTreeSet::<String>::new();
    for active in &snapshot.registries.auth_providers.active {
        let contribution = &active.entry.contribution;
        if !provider_filter.is_none_or(|filter| filter == contribution.provider_id) {
            continue;
        }
        auth_provider_ids.insert(contribution.provider_id.clone());
        rows.push(auth_status_item(active, current_provider));
    }
    for active in &snapshot.registries.providers.active {
        let contribution = &active.entry.contribution;
        if !provider_filter.is_none_or(|filter| filter == contribution.provider_id) {
            continue;
        }
        if contribution.runtime.is_none() && auth_provider_ids.contains(&contribution.provider_id) {
            continue;
        }
        rows.push(runtime_status_item(active, current_provider));
    }
    rows.extend(inactive_status_items(
        snapshot,
        provider_filter,
        current_provider,
    ));
    rows
}

fn inactive_status_items(
    snapshot: &ExtensionManagerSnapshot,
    provider_filter: Option<&str>,
    current_provider: &str,
) -> Vec<AuthStatusItem> {
    let mut rows = Vec::new();
    for inactive in &snapshot.registries.auth_providers.inactive {
        let contribution = &inactive.entry.contribution;
        if provider_filter.is_none_or(|filter| filter == contribution.provider_id) {
            rows.push(inactive_auth_status_item(inactive, current_provider));
        }
    }
    for inactive in &snapshot.registries.providers.inactive {
        let contribution = &inactive.entry.contribution;
        if provider_filter.is_none_or(|filter| filter == contribution.provider_id) {
            rows.push(inactive_runtime_status_item(inactive, current_provider));
        }
    }
    rows
}

fn auth_status_item(
    active: &ActiveContribution<AuthContribution>,
    current_provider: &str,
) -> AuthStatusItem {
    let contribution = &active.entry.contribution;
    let flow = auth_flow_label(contribution.auth_flow);
    let mut detail = format!(
        "Extension auth provider `{}` from {}. Configure through the extension; core Oino does not own provider credentials.",
        active.effective_id,
        contribution_source_summary(active.entry.metadata.provenance.as_ref())
    );
    if let Some(env_var) = &contribution.env_var {
        detail.push_str(&format!(" Env fallback: {env_var}."));
    }
    if let Some(handler) = &contribution.handler {
        detail.push_str(&format!(" Handler: {handler}."));
    }
    AuthStatusItem {
        provider_id: contribution.provider_id.clone(),
        display_name: display_name(&contribution.display_name, &contribution.provider_id),
        auth_kind: flow.into(),
        runtime: "extension auth".into(),
        state: "registered".into(),
        readiness: "extension-managed".into(),
        source: "extension".into(),
        detail,
        setup_url: contribution.setup_url.clone(),
        current: contribution.provider_id == current_provider,
    }
}

fn inactive_auth_status_item(
    inactive: &InactiveContribution<AuthContribution>,
    current_provider: &str,
) -> AuthStatusItem {
    let contribution = &inactive.entry.contribution;
    let reason = inactive.reason.message();
    let remediation = inactive.reason.remediation().unwrap_or_else(|| {
        "inspect extension manager diagnostics or package installation state".into()
    });
    AuthStatusItem {
        provider_id: contribution.provider_id.clone(),
        display_name: display_name(&contribution.display_name, &contribution.provider_id),
        auth_kind: auth_flow_label(contribution.auth_flow).into(),
        runtime: "extension auth".into(),
        state: format!("inactive ({:?})", inactive.reason.health()).to_lowercase(),
        readiness: "inactive".into(),
        source: "extension".into(),
        detail: format!(
            "Extension auth provider `{}` is inactive: {reason}. Remediation: {remediation}.",
            inactive.entry.metadata.id
        ),
        setup_url: contribution.setup_url.clone(),
        current: contribution.provider_id == current_provider,
    }
}

fn runtime_status_item(
    active: &ActiveContribution<ProviderContribution>,
    current_provider: &str,
) -> AuthStatusItem {
    let contribution = &active.entry.contribution;
    let runtime = contribution.runtime.as_ref();
    let readiness = if runtime.is_some() {
        "runtime registered"
    } else if contribution.privacy.can_receive_prompts {
        "metadata only"
    } else {
        "not prompt-capable"
    };
    let detail = runtime.map_or_else(
        || {
            format!(
                "Extension provider `{}` from {} exposes {} seeded model id(s), but no runtime endpoint is registered.",
                active.effective_id,
                contribution_source_summary(active.entry.metadata.provenance.as_ref()),
                contribution.model_ids.len()
            )
        },
        |runtime| {
            format!(
                "Extension runtime provider `{}` from {} uses {} at {} with {} secret policy and {} model id policy. Seeded models: {}.",
                active.effective_id,
                contribution_source_summary(active.entry.metadata.provenance.as_ref()),
                runtime_protocol_label(runtime.protocol),
                runtime.base_url,
                runtime_secret_label(&runtime.api_key),
                runtime_model_policy_label(runtime.model_id),
                contribution.model_ids.len()
            )
        },
    );
    AuthStatusItem {
        provider_id: contribution.provider_id.clone(),
        display_name: display_name(&contribution.display_name, &contribution.provider_id),
        auth_kind: "extension provider".into(),
        runtime: runtime
            .map(|runtime| runtime_protocol_label(runtime.protocol).to_string())
            .unwrap_or_else(|| "extension metadata".into()),
        state: "registered".into(),
        readiness: readiness.into(),
        source: "extension".into(),
        detail,
        setup_url: None,
        current: contribution.provider_id == current_provider,
    }
}

async fn enrich_runtime_status_rows(
    rows: &mut [AuthStatusItem],
    snapshot: &ExtensionManagerSnapshot,
    provider_filter: Option<&str>,
    runtime_detail: &(dyn Fn(&str, &ExtensionRuntimeHealth) -> String + Sync),
) {
    for active in &snapshot.registries.providers.active {
        let contribution = &active.entry.contribution;
        if !provider_filter.is_none_or(|filter| filter == contribution.provider_id) {
            continue;
        }
        let Some(runtime) = contribution.runtime.as_ref() else {
            continue;
        };
        if !rows.iter().any(|row| {
            row.provider_id == contribution.provider_id && row.auth_kind == "extension provider"
        }) {
            continue;
        }
        let health = check_extension_runtime_health(&contribution.provider_id, runtime).await;
        let detail = runtime_detail(&contribution.provider_id, &health);
        for row in rows.iter_mut().filter(|row| {
            row.provider_id == contribution.provider_id && row.auth_kind == "extension provider"
        }) {
            row.readiness = if health.reachable {
                "healthy".into()
            } else {
                "not reachable".into()
            };
            row.source = "extension runtime health".into();
            row.detail.push(' ');
            row.detail.push_str(&detail);
        }
    }
}

fn inactive_runtime_status_item(
    inactive: &InactiveContribution<ProviderContribution>,
    current_provider: &str,
) -> AuthStatusItem {
    let contribution = &inactive.entry.contribution;
    let reason = inactive.reason.message();
    let remediation = inactive.reason.remediation().unwrap_or_else(|| {
        "inspect extension manager diagnostics or package installation state".into()
    });
    let runtime = contribution.runtime.as_ref();
    AuthStatusItem {
        provider_id: contribution.provider_id.clone(),
        display_name: display_name(&contribution.display_name, &contribution.provider_id),
        auth_kind: "extension provider".into(),
        runtime: runtime
            .map(|runtime| runtime_protocol_label(runtime.protocol).to_string())
            .unwrap_or_else(|| "extension metadata".into()),
        state: format!("inactive ({:?})", inactive.reason.health()).to_lowercase(),
        readiness: "inactive".into(),
        source: "extension".into(),
        detail: format!(
            "Extension runtime provider `{}` is inactive: {reason}. Remediation: {remediation}. Seeded models: {}.",
            inactive.entry.metadata.id,
            contribution.model_ids.len()
        ),
        setup_url: None,
        current: contribution.provider_id == current_provider,
    }
}

fn display_name(display_name: &str, provider_id: &str) -> String {
    if display_name.trim().is_empty() {
        provider_id.into()
    } else {
        display_name.into()
    }
}

fn auth_flow_label(flow: AuthFlowType) -> &'static str {
    match flow {
        AuthFlowType::ApiKey => "extension api-key",
        AuthFlowType::OAuth => "extension oauth",
        AuthFlowType::DeviceCode => "extension device-code",
        AuthFlowType::Custom => "extension custom",
    }
}

fn runtime_protocol_label(protocol: ProviderRuntimeProtocol) -> &'static str {
    match protocol {
        ProviderRuntimeProtocol::OpenAiChatCompletions => "OpenAI-compatible chat",
    }
}

fn runtime_secret_label(secret: &ProviderRuntimeSecret) -> String {
    match secret {
        ProviderRuntimeSecret::None => "no".into(),
        ProviderRuntimeSecret::EnvVar { name } => format!("env `{name}`"),
        ProviderRuntimeSecret::ExtensionConfig { key } => format!("extension config `{key}`"),
    }
}

fn runtime_model_policy_label(policy: ProviderRuntimeModelIdPolicy) -> &'static str {
    match policy {
        ProviderRuntimeModelIdPolicy::StripProviderPrefix => "strip-provider-prefix",
        ProviderRuntimeModelIdPolicy::PreserveFullIdentifier => "preserve-full-identifier",
    }
}

fn contribution_source_summary(provenance: Option<&Provenance>) -> String {
    provenance
        .map(|provenance| {
            let package = provenance
                .package_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default();
            let extension = provenance
                .extension_id
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default();
            format!("{} {}", package, extension).trim().to_string()
        })
        .filter(|summary| !summary.is_empty())
        .unwrap_or_else(|| "an extension".into())
}
