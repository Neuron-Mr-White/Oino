//! Metrics, the JSON record + markdown table, and the gate against `bench/budgets.toml`
//! plus a previous record (fail on > 10 % regression or an over-budget result).

use std::fmt::Write;
use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result};
use serde_json::{Value, json};

fn quantile_index(len: usize, percent: usize) -> usize {
    let last = len - 1;
    (last / 100) * percent + ((last % 100) * percent + 50) / 100
}

/// One measured value (a latency distribution or a single number).
#[derive(Clone, Debug)]
pub(crate) struct Metric {
    /// Stable id, e.g. `open.cold.40k.page_ms`.
    pub(crate) id: String,
    /// Budget id from `bench/budgets.toml` (`T3`, …) or empty.
    pub(crate) budget: String,
    /// `ms`, `MB`, `%`, `count`.
    pub(crate) unit: String,
    /// The statistic compared with the budget (`p95`, `p99`, `max`, `value`).
    pub(crate) stat: String,
    pub(crate) n: usize,
    pub(crate) samples: Vec<f64>,
    pub(crate) p50: f64,
    pub(crate) p95: f64,
    pub(crate) p99: f64,
    pub(crate) max: f64,
}

impl Metric {
    /// A latency distribution in milliseconds.
    pub(crate) fn latency(id: &str, budget: &str, stat: &str, samples: &[Duration]) -> Self {
        let mut ms: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
        ms.sort_by(f64::total_cmp);
        let pick = |percent: usize| {
            if ms.is_empty() {
                f64::NAN
            } else {
                ms[quantile_index(ms.len(), percent)]
            }
        };
        Self {
            id: id.to_owned(),
            budget: budget.to_owned(),
            unit: "ms".into(),
            stat: stat.into(),
            n: ms.len(),
            // Keep observation order for adjacent A/B per-sample deltas.
            // Only the percentile working copy above is sorted.
            samples: samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect(),
            p50: pick(50),
            p95: pick(95),
            p99: pick(99),
            max: ms.last().copied().unwrap_or(f64::NAN),
        }
    }

    /// A single value.
    pub(crate) fn value(id: &str, budget: &str, unit: &str, value: f64) -> Self {
        Self {
            id: id.to_owned(),
            budget: budget.to_owned(),
            unit: unit.into(),
            stat: "value".into(),
            n: 1,
            samples: vec![value],
            p50: value,
            p95: value,
            p99: value,
            max: value,
        }
    }

    /// Pool observations, never percentiles of constituent runs. Values use the worst
    /// observation, so a single crash/failure cannot be hidden by averaging.
    pub(crate) fn merge(&mut self, other: &Self) {
        assert_eq!(
            (&self.id, &self.unit, &self.stat),
            (&other.id, &other.unit, &other.stat)
        );
        self.samples.extend_from_slice(&other.samples);
        self.samples.sort_by(f64::total_cmp);
        self.n = self.samples.len();
        let pick = |percent: usize| self.samples[quantile_index(self.n, percent)];
        if self.n > 0 {
            self.p50 = pick(50);
            self.p95 = pick(95);
            self.p99 = pick(99);
            self.max = self.samples[self.n - 1];
        }
    }

    /// The statistic the gate compares.
    pub(crate) fn gated(&self) -> f64 {
        match self.stat.as_str() {
            "p50" => self.p50,
            "p95" => self.p95,
            "p99" => self.p99,
            _ => self.max,
        }
    }

    fn json(&self) -> Value {
        let r = |v: f64| {
            if v.is_finite() {
                json!((v * 100.0).round() / 100.0)
            } else {
                Value::Null
            }
        };
        json!({"id": self.id, "budget": self.budget, "unit": self.unit, "stat": self.stat, "n": self.n, "samples": self.samples,
               "p50": r(self.p50), "p95": r(self.p95), "p99": r(self.p99), "max": r(self.max)})
    }
}

/// A budget row of `bench/budgets.toml`.
#[derive(Clone, Debug)]
pub(crate) struct Budget {
    pub(crate) id: String,
    pub(crate) title: String,
    /// metric id → limit (in the metric's unit).
    pub(crate) limits: Vec<(String, f64)>,
}

/// Parses `bench/budgets.toml`: `[T1]` tables with `title = "…"` and `"<metric id>" = <limit>`.
///
/// # Errors
///
/// Read or parse failures.
pub(crate) fn budgets(path: &Path) -> Result<Vec<Budget>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let table: toml::Table = text
        .parse()
        .with_context(|| format!("parse {}", path.display()))?;
    let mut out = Vec::new();
    for (id, value) in table {
        let Some(row) = value.as_table() else {
            continue;
        };
        let mut budget = Budget {
            id: id.clone(),
            title: String::new(),
            limits: Vec::new(),
        };
        for (k, v) in row {
            match (k.as_str(), v) {
                ("title", toml::Value::String(s)) => budget.title.clone_from(s),
                (metric, toml::Value::Float(f)) => budget.limits.push((metric.to_owned(), *f)),
                (metric, toml::Value::Integer(i)) => {
                    budget
                        .limits
                        .push((metric.to_owned(), crate::signed_float(*i)));
                }
                _ => {}
            }
        }
        out.push(budget);
    }
    Ok(out)
}

/// The limit for a metric id, if any budget names it.
pub(crate) fn limit_for<'a>(budgets: &'a [Budget], id: &str) -> Option<(&'a str, f64)> {
    budgets.iter().find_map(|b| {
        b.limits
            .iter()
            .find(|(m, _)| m == id)
            .map(|(_, l)| (b.id.as_str(), *l))
    })
}

/// One gate verdict.
pub(crate) struct Verdict {
    pub(crate) id: String,
    pub(crate) budget: Option<f64>,
    pub(crate) previous: Option<f64>,
    pub(crate) status: &'static str,
}

/// Over budget → `FAIL budget`; > 10 % (and > 2 units, to absorb noise in tiny numbers)
/// worse than the previous record → `FAIL regression`; else `ok`.
pub(crate) fn judge(
    metrics: &[Metric],
    budgets: &[Budget],
    previous: Option<&Value>,
) -> Vec<Verdict> {
    let mut verdicts: Vec<_> = metrics
        .iter()
        .map(|m| {
            let value = m.gated();
            let budget = limit_for(budgets, &m.id).map(|(_, l)| l);
            let prev = previous.filter(|p| p["quality"] != "noisy").and_then(|p| {
                p["metrics"]
                    .as_array()?
                    .iter()
                    .find(|x| {
                        x["id"] == m.id.as_str()
                            && x["samples"].as_array().is_some_and(|s| {
                                s.len()
                                    >= if matches!(m.stat.as_str(), "p95" | "p99") {
                                        50
                                    } else {
                                        1
                                    }
                            })
                    })
                    .and_then(|x| x[m.stat.as_str()].as_f64().or_else(|| x["max"].as_f64()))
            });
            let status = if matches!(m.stat.as_str(), "p95" | "p99") && m.n < 50 {
                "INSUFFICIENT"
            } else if !value.is_finite() {
                "FAIL missing"
            } else if budget.is_some_and(|b| value > b) {
                "FAIL budget"
            } else if budget.is_some() && prev.is_none() {
                "FAIL baseline"
            } else if prev.is_some_and(|p| value > p * 1.10 && value - p > 2.0) {
                "FAIL regression"
            } else {
                "ok"
            };
            Verdict {
                id: m.id.clone(),
                budget,
                previous: prev,
                status,
            }
        })
        .collect();
    // Every declared limit is mandatory, even for selected/partial suites. An empty
    // budget (T9) declares missing instrumentation and must never pass silently.
    for b in budgets {
        if b.limits.is_empty() {
            verdicts.push(Verdict {
                id: format!("{}.coverage", b.id),
                budget: None,
                previous: None,
                status: "FAIL coverage",
            });
        }
        for (id, limit) in &b.limits {
            if !metrics.iter().any(|m| &m.id == id) {
                verdicts.push(Verdict {
                    id: id.clone(),
                    budget: Some(*limit),
                    previous: None,
                    status: "FAIL coverage",
                });
            }
        }
    }
    verdicts
}

/// Regression-only comparison for an adjacent pair, failing closed on missing A rows.
pub(crate) fn judge_pair(candidate: &[Metric], baseline: &[Metric]) -> Vec<Verdict> {
    let previous = to_json(&json!({}), baseline, &[]);
    let mut verdicts = judge(candidate, &[], Some(&previous));
    for verdict in &mut verdicts {
        if verdict.previous.is_none() && verdict.status == "ok" {
            verdict.status = "FAIL baseline";
        }
    }
    verdicts
}

/// The JSON record.
pub(crate) fn to_json(meta: &Value, metrics: &[Metric], verdicts: &[Verdict]) -> Value {
    let mut out = meta.clone();
    out["metrics"] = metrics.iter().map(Metric::json).collect();
    out["verdicts"] = verdicts
        .iter()
        .map(
            |v| json!({"id": v.id, "budget": v.budget, "previous": v.previous, "status": v.status}),
        )
        .collect();
    out
}

/// The markdown table.
pub(crate) fn to_markdown(meta: &Value, metrics: &[Metric], verdicts: &[Verdict]) -> String {
    let f = |v: f64| {
        if v.is_finite() {
            format!("{v:.1}")
        } else {
            "—".into()
        }
    };
    let mut md = format!(
        "# oino-bench record\n\n- date: {}\n- commit: {}\n- host binary: {}\n- machine: {}\n- scenarios: {}\n\n\
         | metric | T | unit | n | p50 | p95 | p99 | max | gated | budget | status |\n\
         |---|---|---|---|---|---|---|---|---|---|---|",
        meta["date"].as_str().unwrap_or("?"),
        meta["commit"].as_str().unwrap_or("?"),
        meta["host_bin"].as_str().unwrap_or("?"),
        meta["machine"].as_str().unwrap_or("?"),
        meta["scenarios"],
    );
    let quality = format!(
        "- quality: {}\n- quietness start: {}\n- quietness end: {}\n- pooled runs: {}\n\n",
        meta["quality"], meta["quietness_start"], meta["quietness_end"], meta["runs"]
    );
    let Some(table_at) = md.find("| metric") else {
        return md;
    };
    md.insert_str(table_at, &quality);
    for m in metrics {
        let Some(v) = verdicts.iter().find(|v| v.id == m.id) else {
            continue;
        };
        let _ = writeln!(
            md,
            "| {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
            m.id,
            m.budget,
            m.unit,
            m.n,
            f(m.p50),
            f(m.p95),
            f(m.p99),
            f(m.max),
            m.stat,
            v.budget.map_or("—".into(), f),
            v.status
        );
    }
    for v in verdicts
        .iter()
        .filter(|v| !metrics.iter().any(|m| m.id == v.id))
    {
        let _ = writeln!(
            md,
            "| {} | — | — | 0 | — | — | — | — | — | — | {} |",
            v.id, v.status
        );
    }
    if let Some(pairs) = meta["pairs"].as_array() {
        md.push_str("\n## Adjacent A/B comparisons\n\n| scenario | batch | metric | baseline | candidate | status |\n|---|---|---|---|---|---|\n");
        for pair in pairs {
            for verdict in pair["candidate"]["verdicts"]
                .as_array()
                .into_iter()
                .flatten()
            {
                let metric = pair["candidate"]["metrics"]
                    .as_array()
                    .and_then(|ms| ms.iter().find(|m| m["id"] == verdict["id"]));
                let value = metric.and_then(|m| {
                    m[m["stat"].as_str().unwrap_or("max")]
                        .as_f64()
                        .or_else(|| m["max"].as_f64())
                });
                let _ = writeln!(
                    md,
                    "| {} | {} | {} | {} | {} | {} |",
                    pair["scenario"],
                    pair["batch"],
                    verdict["id"],
                    verdict["previous"],
                    value.map_or("—".into(), f),
                    verdict["status"]
                );
            }
        }
    }
    if let Some(notes) = meta["notes"].as_array() {
        md.push_str("\n## Notes\n\n");
        for n in notes {
            let _ = writeln!(md, "- {}", n.as_str().unwrap_or_default());
        }
    }
    md
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentile_indices_round_half_up_without_float_conversion() {
        assert_eq!(quantile_index(1, 99), 0);
        assert_eq!(quantile_index(2, 50), 1);
        assert_eq!(quantile_index(101, 50), 50);
        assert_eq!(quantile_index(101, 95), 95);
        assert_eq!(quantile_index(101, 99), 99);
        assert_eq!(quantile_index(11, 95), 10);
        assert_eq!(quantile_index(usize::MAX, 100), usize::MAX - 1);
    }

    #[test]
    fn latency_keeps_observation_order_for_pair_deltas() {
        let metric = Metric::latency(
            "a",
            "",
            "p95",
            &[
                Duration::from_millis(3),
                Duration::from_millis(1),
                Duration::from_millis(2),
            ],
        );
        assert_eq!(metric.samples, vec![3.0, 1.0, 2.0]);
        assert_eq!(metric.p50, 2.0);
        assert_eq!(metric.p95, 3.0);
        assert_eq!(metric.json()["samples"], json!([3.0, 1.0, 2.0]));
    }

    #[test]
    fn paired_comparison_is_local_and_requires_baseline_samples() {
        let candidate = vec![Metric::value("a", "", "ms", 120.0)];
        assert_eq!(judge_pair(&candidate, &[])[0].status, "FAIL baseline");
        assert_eq!(
            judge_pair(&candidate, &[Metric::value("a", "", "ms", 100.0)])[0].status,
            "FAIL regression"
        );
        assert_eq!(
            judge_pair(&candidate, &[Metric::value("a", "", "ms", 130.0)])[0].status,
            "ok"
        );
        let short = Metric::latency("a", "", "p95", &[Duration::from_millis(1)]);
        let full = Metric::latency("a", "", "p95", &vec![Duration::from_millis(1); 50]);
        assert_eq!(judge_pair(&[full], &[short])[0].status, "FAIL baseline");
    }

    #[test]
    fn insufficient_percentiles_cannot_pass_and_pool_raw_samples() -> Result<()> {
        let mut a = Metric::latency("latency", "T3", "p95", &vec![Duration::from_millis(1); 49]);
        assert_eq!(judge(&[a.clone()], &[], None)[0].status, "INSUFFICIENT");
        let b = Metric::latency("latency", "T3", "p95", &[Duration::from_millis(100)]);
        a.merge(&b);
        assert_eq!(a.n, 50);
        assert_eq!(a.p95, 1.0);
        assert_eq!(a.max, 100.0);
        assert_eq!(judge(&[a.clone()], &[], None)[0].status, "ok");
        let record = to_json(&json!({}), &[a], &[]);
        assert_eq!(
            record["metrics"][0]["samples"]
                .as_array()
                .context("sample array missing")?
                .len(),
            50
        );
        assert_eq!(record["metrics"][0]["samples"][49], 100.0);
        Ok(())
    }

    #[test]
    fn pooling_values_keeps_worst_failure_and_ignores_legacy_baselines() {
        let mut a = Metric::value("failures", "T8", "count", 0.0);
        a.merge(&Metric::value("failures", "T8", "count", 1.0));
        assert_eq!(a.gated(), 1.0);
        let legacy = json!({"metrics": [{"id": "failures", "max": 0.0}]});
        assert!(judge(&[a], &[], Some(&legacy))[0].previous.is_none());
    }

    #[test]
    fn gate_fails_closed_without_baseline_or_coverage() {
        let budgets = vec![
            Budget {
                id: "T1".into(),
                title: String::new(),
                limits: vec![("a".into(), 100.0)],
            },
            Budget {
                id: "T9".into(),
                title: String::new(),
                limits: vec![],
            },
        ];
        let v = judge(&[Metric::value("a", "T1", "ms", 1.0)], &budgets, None);
        assert_eq!(v[0].status, "FAIL baseline");
        assert_eq!(v[1].status, "FAIL coverage");
        assert!(
            judge(&[], &budgets, None)
                .iter()
                .all(|v| v.status == "FAIL coverage")
        );
    }

    #[test]
    fn noisy_baseline_cannot_certify_regression_freedom() {
        let m = Metric::value("a", "T1", "ms", 1.0);
        let b = Budget {
            id: "T1".into(),
            title: String::new(),
            limits: vec![("a".into(), 100.0)],
        };
        let prev = json!({"quality":"noisy", "metrics":[{"id":"a","samples":[1.0],"max":1.0}]});
        assert_eq!(judge(&[m], &[b], Some(&prev))[0].status, "FAIL baseline");
    }

    #[test]
    fn gate_flags_budget_and_regression() {
        let m = vec![
            Metric::value("a", "T1", "ms", 120.0),
            Metric::value("b", "T2", "ms", 50.0),
            Metric::value("c", "", "ms", 10.0),
        ];
        let budgets = vec![Budget {
            id: "T1".into(),
            title: String::new(),
            limits: vec![("a".into(), 100.0)],
        }];
        let prev = json!({"metrics": [{"id": "b", "max": 40.0, "samples": [40.0]}, {"id": "c", "max": 9.5, "samples": [9.5]}]});
        let v = judge(&m, &budgets, Some(&prev));
        assert_eq!(v[0].status, "FAIL budget");
        assert_eq!(v[1].status, "FAIL regression");
        assert_eq!(v[2].status, "ok", "tiny absolute change is noise");
        let lat = Metric::latency(
            "l",
            "",
            "p95",
            &[Duration::from_millis(1), Duration::from_millis(3)],
        );
        assert!((lat.max - 3.0).abs() < 1e-9);
    }
}
