//! App-local provider config surface. Everything `oino-app` needs from
//! `oino-provider-openrouter` is funneled through this module so deleting the
//! native crate (P3c cutover) only touches this file and `provider_runtime.rs`.
//!
//! Config types are re-exports; the OpenRouter usage-report parsing is a copy
//! kept app-side (the wasm providers do not use it).

#[allow(unused_imports)] // some items are only used under cfg(test)
pub use oino_provider_openrouter::{
    CacheControlFormat, OpenAiCompatibleAuth, OpenAiCompatibleConfig, SessionAffinityFormat,
};

use serde_json::Value;

/// OpenRouter `/key` + `/credits` usage report (moved from
/// `oino-provider-openrouter` — same shape, same semantics).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterUsageReport {
    pub limits: Vec<OpenRouterUsageLimit>,
    pub extra_info: Vec<(String, String)>,
    pub balance: Option<OpenRouterCreditBalance>,
    pub hard_limit_reached: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterUsageLimit {
    pub name: String,
    pub usage_percent: f64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterCreditBalance {
    pub amount: f64,
    pub currency: String,
    pub total_credits: f64,
    pub total_usage: f64,
}

#[allow(dead_code)] // exercised by usage tests
pub fn parse_openrouter_usage_payloads(
    key_payload: Option<&str>,
    credits_payload: Option<&str>,
) -> Result<OpenRouterUsageReport, String> {
    let key = key_payload
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|err| format!("invalid key usage fixture: {err}"))?;
    let credits = credits_payload
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|err| format!("invalid credits usage fixture: {err}"))?;
    Ok(parse_openrouter_usage_values(
        key.as_ref(),
        credits.as_ref(),
    ))
}

pub fn parse_openrouter_usage_values(
    key_payload: Option<&Value>,
    credits_payload: Option<&Value>,
) -> OpenRouterUsageReport {
    let mut report = OpenRouterUsageReport {
        limits: Vec::new(),
        extra_info: Vec::new(),
        balance: None,
        hard_limit_reached: false,
    };

    if let Some(data) = credits_payload.and_then(openrouter_data_object) {
        let total_credits = data
            .get("total_credits")
            .and_then(openrouter_f64)
            .unwrap_or(0.0);
        let total_usage = data
            .get("total_usage")
            .and_then(openrouter_f64)
            .unwrap_or(0.0);
        let balance = total_credits - total_usage;
        if total_credits > 0.0 {
            report.limits.push(OpenRouterUsageLimit {
                name: "Credits".into(),
                usage_percent: usage_percent_from_used_limit(total_usage, total_credits),
                resets_at: None,
            });
            report.balance = Some(OpenRouterCreditBalance {
                amount: balance,
                currency: "USD".into(),
                total_credits,
                total_usage,
            });
            report.extra_info.push((
                "Balance".into(),
                format!("${balance:.2} / ${total_credits:.2}"),
            ));
        }
    }

    if let Some(data) = key_payload.and_then(openrouter_data_object) {
        for (label, key) in [
            ("Today", "usage_daily"),
            ("This week", "usage_weekly"),
            ("This month", "usage_monthly"),
        ] {
            if let Some(value) = data.get(key).and_then(openrouter_f64) {
                report
                    .extra_info
                    .push((label.into(), format!("${value:.2}")));
            }
        }
        if let Some(limit) = data
            .get("limit")
            .and_then(openrouter_f64)
            .filter(|value| *value > 0.0)
        {
            let remaining = data
                .get("limit_remaining")
                .and_then(openrouter_f64)
                .unwrap_or(0.0);
            let used = (limit - remaining).max(0.0);
            report.hard_limit_reached = remaining <= 0.0;
            report.limits.push(OpenRouterUsageLimit {
                name: "Key limit".into(),
                usage_percent: usage_percent_from_used_limit(used, limit),
                resets_at: None,
            });
            report.extra_info.push((
                "Key limit".into(),
                format!("${remaining:.2} remaining / ${limit:.2}"),
            ));
        }
    }

    report
}

fn openrouter_data_object(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value.get("data").unwrap_or(value).as_object()
}

fn openrouter_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn usage_percent_from_used_limit(used: f64, limit: f64) -> f64 {
    if !used.is_finite() || !limit.is_finite() || limit <= 0.0 {
        return 0.0;
    }
    (used / limit * 100.0).clamp(0.0, 100.0)
}
