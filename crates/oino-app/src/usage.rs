#![forbid(unsafe_code)]

use crate::{auth_readiness, AppError};
use oino_auth::{AuthStorage, ProviderAuthSpec};
use oino_provider_catalog::{provider_by_id, ProviderDescriptor, ProviderRuntimeSupport};
use oino_provider_openrouter::{OpenRouterUsageReport, OPENROUTER_PROVIDER_ID};
use oino_tui::{UsagePanelProvider, UsagePanelReport, UsagePanelSession};
use oino_types::{Message, Usage, UsageCost};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct UsageReport {
    pub generated_at_unix: u64,
    pub session: SessionUsageSummary,
    pub providers: Vec<ProviderUsageProgress>,
}

impl UsageReport {
    #[must_use]
    pub(crate) fn from_messages(messages: &[Message]) -> Self {
        build_usage_report(messages, now_unix())
    }

    pub(crate) fn upsert_provider_progress(&mut self, progress: ProviderUsageProgress) {
        if let Some(existing) = self
            .providers
            .iter_mut()
            .find(|item| item.provider_id == progress.provider_id)
        {
            if existing.session_usage.is_some() {
                existing.account_usage = progress.account_usage;
                if !progress.message.trim().is_empty() {
                    existing.message = format!("{}; {}", existing.message, progress.message);
                }
            } else {
                *existing = progress;
            }
            return;
        }
        self.providers.push(progress);
        self.providers
            .sort_by(|left, right| left.display_name.cmp(&right.display_name));
    }

    #[must_use]
    pub(crate) fn to_tui_report(&self) -> UsagePanelReport {
        UsagePanelReport {
            generated_at_unix: self.generated_at_unix,
            status_line: self.status_line(),
            session: UsagePanelSession {
                assistant_turns: self.session.assistant_turns,
                reported_turns: self.session.reported_turns,
                input_tokens: self.session.totals.input_tokens,
                output_tokens: self.session.totals.output_tokens,
                cache_read_tokens: self.session.totals.cache_read_tokens,
                cache_write_tokens: self.session.totals.cache_write_tokens,
                total_tokens: self.session.totals.total_tokens(),
                costs: formatted_costs(&self.session.costs),
            },
            providers: self
                .providers
                .iter()
                .map(provider_progress_to_tui)
                .collect(),
        }
    }

    #[must_use]
    pub(crate) fn format_text(&self) -> String {
        let mut lines = vec![self.status_line()];
        lines.push(format!(
            "Session: {} assistant turn(s), {} reported turn(s), {} input / {} output / {} cache read / {} cache write tokens",
            self.session.assistant_turns,
            self.session.reported_turns,
            compact_count(self.session.totals.input_tokens),
            compact_count(self.session.totals.output_tokens),
            compact_count(self.session.totals.cache_read_tokens),
            compact_count(self.session.totals.cache_write_tokens),
        ));
        let costs = format_costs(&self.session.costs);
        if !costs.is_empty() {
            lines.push(format!("Session cost: {costs}"));
        }
        if self.providers.is_empty() {
            lines.push("Providers: no provider usage rows yet".into());
        } else {
            lines.push("Providers:".into());
            for provider in &self.providers {
                lines.push(format!(
                    "- {} ({}): {}",
                    provider.display_name, provider.provider_id, provider.message
                ));
                if let Some(session) = &provider.session_usage {
                    lines.push(format!(
                        "  session: {} assistant turn(s), {} reported turn(s), {} tokens",
                        session.assistant_turns,
                        session.reported_turns,
                        compact_count(session.totals.total_tokens())
                    ));
                }
                if let Some(account) = &provider.account_usage {
                    lines.push(format!(
                        "  account: {} refreshed at {}",
                        account.source, account.refreshed_at_unix
                    ));
                    if let Some(balance) = &account.balance {
                        lines.push(format!(
                            "  balance: {:.4} {}",
                            balance.amount, balance.currency
                        ));
                    }
                    for limit in &account.limits {
                        lines.push(format!("  limit: {}", format_usage_limit(limit)));
                    }
                }
            }
        }
        lines.join("\n")
    }

    #[must_use]
    pub(crate) fn status_line(&self) -> String {
        if self.session.reported_turns == 0 {
            return "Usage: no provider token reports in this session yet".into();
        }
        let total = self.session.totals.total_tokens();
        let costs = format_costs(&self.session.costs);
        if costs.is_empty() {
            format!(
                "Usage: {} reported turn(s), {} tokens",
                self.session.reported_turns,
                compact_count(total)
            )
        } else {
            format!(
                "Usage: {} reported turn(s), {} tokens, {costs}",
                self.session.reported_turns,
                compact_count(total)
            )
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct SessionUsageSummary {
    pub assistant_turns: u64,
    pub reported_turns: u64,
    pub totals: TokenUsageTotals,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub costs: Vec<UsageCostTotal>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub by_provider: Vec<ProviderSessionUsage>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TokenUsageTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

impl TokenUsageTotals {
    fn add_usage(&mut self, usage: &Usage) {
        self.input_tokens = self.input_tokens.saturating_add(usage.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(usage.output_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(usage.cache_read_tokens);
        self.cache_write_tokens = self
            .cache_write_tokens
            .saturating_add(usage.cache_write_tokens);
    }

    #[must_use]
    pub(crate) const fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_read_tokens + self.cache_write_tokens
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct UsageCostTotal {
    pub amount: f64,
    pub currency: String,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProviderSessionUsage {
    pub provider_id: String,
    pub display_name: String,
    pub assistant_turns: u64,
    pub reported_turns: u64,
    pub totals: TokenUsageTotals,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub costs: Vec<UsageCostTotal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ProviderUsageStatus {
    Loading,
    Available,
    NoData,
    NotConfigured,
    Unsupported,
    Error,
    Info,
}

impl ProviderUsageStatus {
    #[must_use]
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Loading => "loading",
            Self::Available => "available",
            Self::NoData => "no data",
            Self::NotConfigured => "not configured",
            Self::Unsupported => "unsupported",
            Self::Error => "error",
            Self::Info => "info",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProviderUsageProgress {
    pub provider_id: String,
    pub display_name: String,
    pub status: ProviderUsageStatus,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_usage: Option<ProviderSessionUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_usage: Option<ProviderUsage>,
    pub updated_at_unix: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProviderUsage {
    pub source: String,
    pub refreshed_at_unix: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub limits: Vec<UsageLimit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<UsageCostTotal>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct UsageLimit {
    pub label: String,
    pub used: f64,
    pub limit: Option<f64>,
    pub unit: String,
    pub reset_at_unix: Option<u64>,
}

pub(crate) async fn account_usage_progress_placeholder(
    auth: &AuthStorage,
    provider: ProviderDescriptor,
    updated_at_unix: u64,
) -> Result<ProviderUsageProgress, AppError> {
    let credential = provider.credential_spec();
    let (status, message) = match provider.runtime {
        ProviderRuntimeSupport::OpenAiCompatible => {
            if !credential.requires_api_key {
                (
                    ProviderUsageStatus::Info,
                    format!(
                        "account usage: {} does not require an API key; showing session totals only",
                        provider.display_name
                    ),
                )
            } else if let Some(env_var) = credential.env_var {
                let spec = ProviderAuthSpec::new(credential.provider_id, credential.auth_key, env_var);
                let assessment = auth.assess_api_key_provider(&spec).await?;
                if assessment.is_configured() {
                    (
                        ProviderUsageStatus::Info,
                        format!(
                            "account usage fetch for {} is staged; showing session totals from provider responses",
                            provider.display_name
                        ),
                    )
                } else {
                    (
                        ProviderUsageStatus::NotConfigured,
                        format!(
                            "account usage unavailable until {} is configured; use `/9router setup` and select a `9router:<model>` model, or install an extension account-usage provider. Historical env var: {}",
                            provider.display_name, env_var
                        ),
                    )
                }
            } else {
                (
                    ProviderUsageStatus::Unsupported,
                    format!(
                        "account usage is not available for {} yet; no API-key credential spec is registered",
                        provider.display_name
                    ),
                )
            }
        }
        ProviderRuntimeSupport::Native => {
            let removed_detail = auth_readiness::removed_provider_runtime_detail(provider);
            if let Some(env_var) = credential.env_var {
                let spec = ProviderAuthSpec::new(credential.provider_id, credential.auth_key, env_var);
                let assessment = auth.assess_api_key_provider(&spec).await?;
                if assessment.is_configured() {
                    (
                        ProviderUsageStatus::Unsupported,
                        format!(
                            "account usage for {} is not available from core; direct provider runtime was removed. {removed_detail}",
                            provider.display_name
                        ),
                    )
                } else if credential.requires_api_key {
                    (
                        ProviderUsageStatus::NotConfigured,
                        format!(
                            "account usage unavailable for {} from core; use `/9router setup` and select a `9router:<model>` model, or install an extension account-usage provider. Historical env var: {}. {removed_detail}",
                            provider.display_name, env_var
                        ),
                    )
                } else {
                    (
                        ProviderUsageStatus::Unsupported,
                        format!(
                            "account usage for {} is not available from core. {removed_detail}",
                            provider.display_name
                        ),
                    )
                }
            } else {
                (
                    ProviderUsageStatus::Unsupported,
                    format!(
                        "account usage for {} is not available from core. {removed_detail}",
                        provider.display_name
                    ),
                )
            }
        }
        ProviderRuntimeSupport::MetadataOnly | ProviderRuntimeSupport::Import => (
            ProviderUsageStatus::Unsupported,
            format!(
                "{} is represented for setup/status only; session totals appear when a runtime adapter is used",
                provider.display_name
            ),
        ),
    };
    Ok(ProviderUsageProgress {
        provider_id: provider.id.into(),
        display_name: provider.display_name.into(),
        status,
        message,
        session_usage: None,
        account_usage: None,
        updated_at_unix,
    })
}

#[derive(Debug, Default)]
struct ProviderAccumulator {
    display_name: String,
    assistant_turns: u64,
    reported_turns: u64,
    totals: TokenUsageTotals,
    costs: BTreeMap<String, f64>,
}

fn build_usage_report(messages: &[Message], generated_at_unix: u64) -> UsageReport {
    let mut session = SessionUsageSummary::default();
    let mut session_costs = BTreeMap::<String, f64>::new();
    let mut providers = BTreeMap::<String, ProviderAccumulator>::new();

    for message in messages {
        let Message::Assistant {
            usage, provider, ..
        } = message
        else {
            continue;
        };
        session.assistant_turns = session.assistant_turns.saturating_add(1);
        let provider_id = provider
            .as_ref()
            .and_then(|metadata| metadata.model.as_ref())
            .map_or_else(|| "unknown".to_string(), |model| model.provider.clone());
        let provider_entry =
            providers
                .entry(provider_id.clone())
                .or_insert_with(|| ProviderAccumulator {
                    display_name: provider_display_name(&provider_id),
                    ..ProviderAccumulator::default()
                });
        provider_entry.assistant_turns = provider_entry.assistant_turns.saturating_add(1);
        let Some(usage) = usage else {
            continue;
        };
        session.reported_turns = session.reported_turns.saturating_add(1);
        session.totals.add_usage(usage);
        provider_entry.reported_turns = provider_entry.reported_turns.saturating_add(1);
        provider_entry.totals.add_usage(usage);
        if let Some(cost) = &usage.cost {
            add_cost(&mut session_costs, cost);
            add_cost(&mut provider_entry.costs, cost);
        }
    }

    session.costs = sorted_costs(session_costs);
    session.by_provider = providers
        .iter()
        .map(|(provider_id, entry)| provider_session_usage(provider_id, entry))
        .collect();
    let provider_progress = session
        .by_provider
        .iter()
        .cloned()
        .map(|usage| {
            let status = if usage.reported_turns == 0 {
                ProviderUsageStatus::NoData
            } else {
                ProviderUsageStatus::Available
            };
            let message = if usage.reported_turns == 0 {
                format!(
                    "{}: {} assistant turn(s), but no provider token usage reported yet",
                    status.label(),
                    usage.assistant_turns
                )
            } else {
                format!(
                    "{}: {} reported turn(s), {} tokens",
                    status.label(),
                    usage.reported_turns,
                    compact_count(usage.totals.total_tokens())
                )
            };
            ProviderUsageProgress {
                provider_id: usage.provider_id.clone(),
                display_name: usage.display_name.clone(),
                status,
                message,
                session_usage: Some(usage),
                account_usage: None,
                updated_at_unix: generated_at_unix,
            }
        })
        .collect();

    UsageReport {
        generated_at_unix,
        session,
        providers: provider_progress,
    }
}

fn provider_session_usage(provider_id: &str, entry: &ProviderAccumulator) -> ProviderSessionUsage {
    ProviderSessionUsage {
        provider_id: provider_id.into(),
        display_name: entry.display_name.clone(),
        assistant_turns: entry.assistant_turns,
        reported_turns: entry.reported_turns,
        totals: entry.totals.clone(),
        costs: sorted_costs(entry.costs.clone()),
    }
}

fn provider_display_name(provider_id: &str) -> String {
    provider_by_id(provider_id).map_or_else(
        || provider_id.to_string(),
        |provider| provider.display_name.to_string(),
    )
}

fn add_cost(costs: &mut BTreeMap<String, f64>, cost: &UsageCost) {
    *costs.entry(cost.currency.clone()).or_default() += cost.amount;
}

fn sorted_costs(costs: BTreeMap<String, f64>) -> Vec<UsageCostTotal> {
    costs
        .into_iter()
        .map(|(currency, amount)| UsageCostTotal { amount, currency })
        .collect()
}

#[allow(dead_code)] // Staged no-live account-usage seam; covered by fixture tests until a fetcher calls it.
pub(crate) fn openrouter_usage_report_to_provider_usage(
    report: &OpenRouterUsageReport,
    refreshed_at_unix: u64,
) -> ProviderUsage {
    let source = if report.extra_info.is_empty() {
        "OpenRouter account usage".to_string()
    } else {
        let info = report
            .extra_info
            .iter()
            .map(|(key, value)| format!("{key}: {value}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!("OpenRouter account usage ({info})")
    };
    ProviderUsage {
        source,
        refreshed_at_unix,
        limits: report
            .limits
            .iter()
            .map(|limit| UsageLimit {
                label: limit.name.clone(),
                used: limit.usage_percent,
                limit: Some(100.0),
                unit: "%".into(),
                reset_at_unix: limit
                    .resets_at
                    .as_deref()
                    .and_then(parse_reset_timestamp_hint),
            })
            .collect(),
        balance: report.balance.as_ref().map(|balance| UsageCostTotal {
            amount: balance.amount,
            currency: balance.currency.clone(),
        }),
    }
}

#[allow(dead_code)] // Staged no-live account-usage seam; covered by fixture tests until a fetcher calls it.
pub(crate) fn openrouter_usage_report_to_provider_progress(
    report: &OpenRouterUsageReport,
    refreshed_at_unix: u64,
) -> ProviderUsageProgress {
    let account_usage = openrouter_usage_report_to_provider_usage(report, refreshed_at_unix);
    let status = if report.hard_limit_reached {
        ProviderUsageStatus::Error
    } else if report.limits.is_empty() && report.extra_info.is_empty() && report.balance.is_none() {
        ProviderUsageStatus::NoData
    } else {
        ProviderUsageStatus::Available
    };
    let mut parts = Vec::new();
    if report.hard_limit_reached {
        parts.push("hard limit reached".to_string());
    }
    if report.limits.is_empty() {
        parts.push("no usage limits in fixture".to_string());
    } else {
        parts.push(format!("{} usage limit(s) parsed", report.limits.len()));
    }
    for (key, value) in &report.extra_info {
        parts.push(format!("{key}: {value}"));
    }
    ProviderUsageProgress {
        provider_id: OPENROUTER_PROVIDER_ID.into(),
        display_name: provider_display_name(OPENROUTER_PROVIDER_ID),
        status,
        message: format!("OpenRouter account usage: {}", parts.join("; ")),
        session_usage: None,
        account_usage: Some(account_usage),
        updated_at_unix: refreshed_at_unix,
    }
}

#[allow(dead_code)] // Used by staged usage conversion helpers above.
fn parse_reset_timestamp_hint(raw: &str) -> Option<u64> {
    raw.trim().parse::<u64>().ok()
}

fn provider_progress_to_tui(progress: &ProviderUsageProgress) -> UsagePanelProvider {
    let session = progress.session_usage.as_ref();
    let account = progress.account_usage.as_ref();
    UsagePanelProvider {
        provider_id: progress.provider_id.clone(),
        display_name: progress.display_name.clone(),
        status: progress.status.label().into(),
        message: progress.message.clone(),
        assistant_turns: session.map_or(0, |usage| usage.assistant_turns),
        reported_turns: session.map_or(0, |usage| usage.reported_turns),
        total_tokens: session.map_or(0, |usage| usage.totals.total_tokens()),
        costs: session.map_or_else(Vec::new, |usage| formatted_costs(&usage.costs)),
        account_source: account.map(|usage| {
            format!(
                "{} • refreshed at {}",
                usage.source, usage.refreshed_at_unix
            )
        }),
        account_balance: account
            .and_then(|usage| usage.balance.as_ref())
            .map(format_cost),
        account_limits: account.map_or_else(Vec::new, |usage| {
            usage.limits.iter().map(format_usage_limit).collect()
        }),
    }
}

fn format_cost(cost: &UsageCostTotal) -> String {
    format!("{:.4} {}", cost.amount, cost.currency)
}

fn format_usage_limit(limit: &UsageLimit) -> String {
    let value = match limit.limit {
        Some(total) => format!("{:.2}/{:.2}", limit.used, total),
        None => format!("{:.2}", limit.used),
    };
    let reset = limit
        .reset_at_unix
        .map_or(String::new(), |reset| format!(" • resets at {reset}"));
    format!("{}: {value} {}{reset}", limit.label, limit.unit)
}

fn formatted_costs(costs: &[UsageCostTotal]) -> Vec<String> {
    costs.iter().map(format_cost).collect()
}

fn format_costs(costs: &[UsageCostTotal]) -> String {
    formatted_costs(costs).join(", ")
}

fn compact_count(value: u64) -> String {
    if value >= 1_000_000 {
        compact_scaled_count(value, 1_000_000, "m")
    } else if value >= 1_000 {
        compact_scaled_count(value, 1_000, "k")
    } else {
        value.to_string()
    }
}

fn compact_scaled_count(value: u64, scale: u64, suffix: &str) -> String {
    let whole = value / scale;
    let frac = (value % scale) * 10 / scale;
    if frac == 0 {
        format!("{whole}{suffix}")
    } else {
        format!("{whole}.{frac}{suffix}")
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_provider_openrouter::parse_openrouter_usage_payloads;
    use oino_types::{ContentBlock, Model, OinoId, ProviderMetadata, StopReason};

    fn assistant(provider: Option<&str>, usage: Option<Usage>) -> Message {
        Message::Assistant {
            id: OinoId::nil(),
            content: vec![ContentBlock::Text { text: "ok".into() }],
            stop_reason: Some(StopReason::EndTurn),
            usage,
            provider: provider.map(|provider| ProviderMetadata {
                request_id: None,
                model: Some(Model::new(provider, "test-model")),
                values: Default::default(),
            }),
        }
    }

    #[test]
    fn summarizes_session_usage_by_provider() {
        let report = build_usage_report(
            &[
                assistant(
                    Some("openrouter"),
                    Some(Usage {
                        input_tokens: 10,
                        output_tokens: 20,
                        cache_read_tokens: 3,
                        cache_write_tokens: 2,
                        cost: Some(UsageCost {
                            amount: 0.001,
                            currency: "USD".into(),
                        }),
                    }),
                ),
                assistant(
                    Some("deepseek"),
                    Some(Usage {
                        input_tokens: 5,
                        output_tokens: 7,
                        cache_read_tokens: 0,
                        cache_write_tokens: 0,
                        cost: Some(UsageCost {
                            amount: 0.002,
                            currency: "USD".into(),
                        }),
                    }),
                ),
            ],
            123,
        );

        assert_eq!(report.session.assistant_turns, 2);
        assert_eq!(report.session.reported_turns, 2);
        assert_eq!(report.session.totals.total_tokens(), 47);
        assert_eq!(report.session.costs[0].amount, 0.003);
        assert_eq!(report.session.by_provider.len(), 2);
        assert!(report.status_line().contains("47 tokens"));
    }

    #[test]
    fn records_no_data_provider_progress_for_missing_usage() {
        let report = build_usage_report(&[assistant(Some("openrouter"), None)], 123);

        assert_eq!(report.session.assistant_turns, 1);
        assert_eq!(report.session.reported_turns, 0);
        assert_eq!(report.providers[0].status, ProviderUsageStatus::NoData);
        assert_eq!(report.providers[0].status.label(), "no data");
        assert!(report.status_line().contains("no provider token reports"));
    }

    #[test]
    fn upserts_provider_progress_without_losing_session_usage() {
        let mut report = build_usage_report(&[assistant(Some("openrouter"), None)], 123);
        report.upsert_provider_progress(ProviderUsageProgress {
            provider_id: "openrouter".into(),
            display_name: "OpenRouter".into(),
            status: ProviderUsageStatus::Info,
            message: "account usage fetch pending".into(),
            session_usage: None,
            account_usage: None,
            updated_at_unix: 123,
        });

        assert_eq!(report.providers.len(), 1);
        assert!(report.providers[0].session_usage.is_some());
        assert!(report.providers[0]
            .message
            .contains("account usage fetch pending"));
    }

    #[test]
    fn converts_to_tui_report_with_formatted_costs() {
        let report = build_usage_report(
            &[assistant(
                Some("openrouter"),
                Some(Usage {
                    input_tokens: 1,
                    output_tokens: 2,
                    cache_read_tokens: 3,
                    cache_write_tokens: 4,
                    cost: Some(UsageCost {
                        amount: 0.125,
                        currency: "USD".into(),
                    }),
                }),
            )],
            123,
        );
        let tui = report.to_tui_report();

        assert_eq!(tui.generated_at_unix, 123);
        assert_eq!(tui.session.total_tokens, 10);
        assert_eq!(tui.session.costs, vec!["0.1250 USD"]);
        assert_eq!(tui.providers[0].total_tokens, 10);
    }

    #[test]
    fn converts_openrouter_usage_fixture_to_provider_usage() {
        let parsed = parse_openrouter_usage_payloads(
            Some(
                r#"{
                  "data": {
                    "usage_daily": 1.25,
                    "limit": 10.0,
                    "limit_remaining": 4.0
                  }
                }"#,
            ),
            Some(
                r#"{
                  "data": {
                    "total_credits": 20.0,
                    "total_usage": 5.0
                  }
                }"#,
            ),
        )
        .unwrap_or_else(|err| panic!("parse fixture failed: {err}"));

        let progress = openrouter_usage_report_to_provider_progress(&parsed, 1234);
        assert_eq!(progress.provider_id, "openrouter");
        assert_eq!(progress.display_name, "OpenRouter");
        assert_eq!(progress.status, ProviderUsageStatus::Available);
        assert!(progress.message.contains("2 usage limit(s) parsed"));
        assert!(progress.message.contains("Balance: $15.00 / $20.00"));
        let account = progress
            .account_usage
            .as_ref()
            .unwrap_or_else(|| panic!("account usage missing"));
        assert_eq!(account.balance.as_ref().map(|cost| cost.amount), Some(15.0));
        assert!(account.source.contains("Today: $1.25"));
        assert_eq!(account.limits[0].label, "Credits");
        assert_eq!(account.limits[0].used, 25.0);
        assert_eq!(account.limits[1].label, "Key limit");
        assert_eq!(account.limits[1].used, 60.0);
    }

    #[test]
    fn converts_openrouter_hard_limit_fixture_to_error_progress() {
        let parsed = parse_openrouter_usage_payloads(
            Some(r#"{"data":{"limit":5,"limit_remaining":0}}"#),
            None,
        )
        .unwrap_or_else(|err| panic!("parse fixture failed: {err}"));

        let progress = openrouter_usage_report_to_provider_progress(&parsed, 77);
        assert_eq!(progress.status, ProviderUsageStatus::Error);
        assert!(progress.message.contains("hard limit reached"));
        let account = progress.account_usage.expect("account usage");
        assert_eq!(account.limits[0].used, 100.0);
        assert_eq!(
            format_usage_limit(&account.limits[0]),
            "Key limit: 100.00/100.00 %"
        );
    }

    #[test]
    fn converts_account_usage_to_tui_provider_details() {
        let mut report = build_usage_report(&[], 123);
        report.upsert_provider_progress(ProviderUsageProgress {
            provider_id: "openrouter".into(),
            display_name: "OpenRouter".into(),
            status: ProviderUsageStatus::Available,
            message: "account usage available".into(),
            session_usage: None,
            account_usage: Some(ProviderUsage {
                source: "fixture".into(),
                refreshed_at_unix: 456,
                limits: vec![UsageLimit {
                    label: "daily tokens".into(),
                    used: 25.0,
                    limit: Some(100.0),
                    unit: "tokens".into(),
                    reset_at_unix: Some(789),
                }],
                balance: Some(UsageCostTotal {
                    amount: 1.5,
                    currency: "USD".into(),
                }),
            }),
            updated_at_unix: 456,
        });
        let tui = report.to_tui_report();
        let provider = &tui.providers[0];

        assert_eq!(
            provider.account_source.as_deref(),
            Some("fixture • refreshed at 456")
        );
        assert_eq!(provider.account_balance.as_deref(), Some("1.5000 USD"));
        assert_eq!(
            provider.account_limits,
            vec!["daily tokens: 25.00/100.00 tokens • resets at 789"]
        );
        assert!(report.format_text().contains("balance: 1.5000 USD"));
    }
}
