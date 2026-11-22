//! Usage over every session on this machine (unipi `usage-parser.ts`).
//!
//! Per session file, compact records `[ts, hashTokens, countedTokens, cost, model, counted]`
//! plus compaction-savings rows are cached by `(modified, size)` in `/cache/usage-stats.json`;
//! only changed files are re-read (`session-read`). Aggregation dedups records across files by
//! `(timestamp, hashTokens)` (forks copy history) and compactions by `(at, before)`.
//! Periods: today, this week (Monday), this month, 30 daily buckets — in UTC until the host
//! exposes the local offset (see deviations).

use std::collections::{BTreeMap, HashMap, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use unipi_kit::savings::{Accumulator, CompactionSaving, Totals, totals_of};

/// Cache format version.
pub const CACHE_VERSION: u32 = 2;
/// Days in `daily`.
pub const DAILY_DAYS: usize = 30;
const DAY_MS: i64 = 86_400_000;

/// `[timestamp, hashTokens, countedTokens, cost, modelIndex, counted]`.
pub type Record = (i64, f64, f64, f64, usize, u8);
/// `[at, before, after, replies, paid, context]`.
pub type SavingRow = (i64, f64, f64, u64, f64, f64);

/// Per-file cache entry.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedFile {
    /// Modification stamp.
    pub mtime_ms: i64,
    /// Size stamp.
    pub size: u64,
    /// Records.
    pub records: Vec<Record>,
    /// Compaction rows.
    #[serde(default)]
    pub savings: Vec<SavingRow>,
}

/// The cache file.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CacheFile {
    /// Format version.
    pub version: u32,
    /// Interned model names.
    pub models: Vec<String>,
    /// Per path.
    pub files: BTreeMap<String, CachedFile>,
}

/// Per-model bucket.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelUse {
    /// Tokens.
    pub tokens: f64,
    /// Cost.
    pub cost: f64,
    /// Records counted.
    pub sessions: u64,
}

/// Aggregated stats (unipi `UsageStats`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageStats {
    /// Tokens today/week/month.
    pub tokens: Periods,
    /// Cost today / all time.
    pub cost: CostPeriods,
    /// Per model, all time.
    pub by_model: BTreeMap<String, ModelUse>,
    /// Per model, this week.
    pub by_model_week: BTreeMap<String, ModelUse>,
    /// Per model, this month.
    pub by_model_month: BTreeMap<String, ModelUse>,
    /// Sessions with usage.
    pub session_count: u64,
    /// Last 30 days, oldest → today.
    pub daily: Vec<Day>,
    /// Sessions with usage today.
    pub sessions_today: u64,
    /// Compaction savings, all sessions.
    pub compaction: SavingsTotals,
    /// Same, per session directory (= project).
    pub compaction_by_dir: BTreeMap<String, SavingsTotals>,
}

/// Token periods.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Periods {
    /// Today.
    pub today: f64,
    /// This week.
    pub week: f64,
    /// This month.
    pub month: f64,
}

/// Cost periods.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CostPeriods {
    /// Today.
    pub today: f64,
    /// All time.
    pub all_time: f64,
}

/// One day.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Day {
    /// Tokens.
    pub tokens: f64,
    /// Cost.
    pub cost: f64,
}

/// Serializable savings totals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SavingsTotals {
    /// Compactions.
    pub count: usize,
    /// Saved tokens.
    pub saved: f64,
    /// Tokens not re-sent.
    pub avoided: f64,
    /// Money not spent.
    pub dollars: f64,
    /// Context sent after compactions.
    pub sent: f64,
}

impl From<Totals> for SavingsTotals {
    fn from(t: Totals) -> Self {
        Self {
            count: t.count,
            saved: t.saved,
            avoided: t.avoided,
            dollars: t.dollars,
            sent: t.sent,
        }
    }
}

fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).unwrap_or(0.0)
}

fn intern(models: &mut Vec<String>, name: &str) -> usize {
    models.iter().position(|m| m == name).unwrap_or_else(|| {
        models.push(name.to_owned());
        models.len() - 1
    })
}

fn entry_ts(e: &Value) -> i64 {
    e.get("timestamp")
        .and_then(Value::as_str)
        .and_then(crate::time::parse_iso_ms)
        .unwrap_or(0)
}

/// Extracts records and savings rows from one session's entries (no dedup here).
pub fn extract(entries: &[Value], models: &mut Vec<String>) -> (Vec<Record>, Vec<SavingRow>) {
    let mut records = Vec::new();
    let mut acc = Accumulator::default();
    for e in entries {
        acc.feed(e);
        let kind = e.get("type").and_then(Value::as_str);
        // Child-agent spend recorded into a lead session: unipi custom entries and Oino `usage`.
        let child = match kind {
            Some("custom")
                if e.get("customType").and_then(Value::as_str)
                    == Some(crate::CHILD_USAGE_ENTRY) =>
            {
                let d = e.get("data").cloned().unwrap_or(Value::Null);
                if d.get("standalone").and_then(Value::as_bool) == Some(false) {
                    continue;
                }
                let model = d
                    .get("model")
                    .and_then(Value::as_str)
                    .map(|m| m.split_once('/').map_or(m, |(_, id)| id).to_owned());
                Some((d.clone(), model))
            }
            Some("usage") => Some((
                e.get("usage").cloned().unwrap_or(Value::Null),
                e.get("model")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
            )),
            _ => None,
        };
        if let Some((u, model)) = child {
            let (i, o, cr, cw) = (
                num(u.get("input")),
                num(u.get("output")),
                num(u.get("cacheRead")),
                num(u.get("cacheWrite")),
            );
            let cost = u
                .get("cost")
                .map_or(0.0, |c| c.as_f64().unwrap_or_else(|| num(c.get("total"))));
            let idx = intern(models, model.as_deref().unwrap_or("child-agent"));
            records.push((
                entry_ts(e),
                i + o + cr + cw,
                i + o + cw,
                cost,
                idx,
                u8::from(i > 0.0 || o > 0.0 || cost > 0.0),
            ));
            continue;
        }
        if kind != Some("message") {
            continue;
        }
        let Some(m) = e
            .get("message")
            .filter(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
        else {
            continue;
        };
        let (Some(u), Some(_), Some(model)) = (
            m.get("usage"),
            m.get("provider"),
            m.get("model").and_then(Value::as_str),
        ) else {
            continue;
        };
        let (i, o, cr, cw) = (
            num(u.get("input")),
            num(u.get("output")),
            num(u.get("cacheRead")),
            num(u.get("cacheWrite")),
        );
        let cost = num(u.pointer("/cost/total"));
        let ts = m
            .get("timestamp")
            .and_then(Value::as_i64)
            .filter(|t| *t != 0)
            .unwrap_or_else(|| entry_ts(e));
        let idx = intern(models, model);
        records.push((
            ts,
            i + o + cr + cw,
            i + o + cw,
            cost,
            idx,
            u8::from(i > 0.0 || o > 0.0 || cost > 0.0),
        ));
    }
    let rows = acc
        .items
        .iter()
        .map(|c| (c.at, c.before, c.after, c.replies, c.paid, c.context))
        .collect();
    (records, rows)
}

/// Period starts in UTC: today 00:00, Monday 00:00, the 1st 00:00, and the 30 day starts.
#[must_use]
pub fn bounds(now: i64) -> (i64, i64, i64, Vec<i64>) {
    let today = now.div_euclid(DAY_MS) * DAY_MS;
    let days_since_epoch = now.div_euclid(DAY_MS);
    // 1970-01-01 was a Thursday: Monday-based weekday = (days + 3) % 7.
    let weekday = (days_since_epoch + 3).rem_euclid(7);
    let week = today - weekday * DAY_MS;
    let (y, m, _) = civil(days_since_epoch);
    let month = days_from_civil(y, m, 1) * DAY_MS;
    #[allow(clippy::cast_possible_wrap, reason = "30 days")]
    let days = (0..DAILY_DAYS as i64)
        .map(|i| today - (DAILY_DAYS as i64 - 1 - i) * DAY_MS)
        .collect();
    (today, week, month, days)
}

/// Civil date of days since the epoch.
#[must_use]
pub fn civil(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// Aggregates cached files (in path order) at `now`.
#[must_use]
pub fn aggregate(cache: &CacheFile, now: i64) -> UsageStats {
    let (today, week, month, days) = bounds(now);
    let mut s = UsageStats {
        daily: vec![Day::default(); DAILY_DAYS],
        ..UsageStats::default()
    };
    let mut seen: HashMap<i64, Vec<u64>> = HashMap::new();
    let mut seen_compactions: HashSet<(i64, u64)> = HashSet::new();
    let mut all = Vec::new();
    let mut by_dir: BTreeMap<String, Vec<CompactionSaving>> = BTreeMap::new();
    let bump = |b: &mut BTreeMap<String, ModelUse>, model: &str, t: f64, c: f64| {
        let e = b.entry(model.to_owned()).or_default();
        e.tokens += t;
        e.cost += c;
        e.sessions += 1;
    };
    for (path, file) in &cache.files {
        let (mut counted, mut counted_today) = (0u64, false);
        for &(ts, hash, tokens, cost, model, is_counted) in &file.records {
            let hashes = seen.entry(ts).or_default();
            if hashes.contains(&hash.to_bits()) {
                continue;
            }
            hashes.push(hash.to_bits());
            if is_counted == 0 {
                continue;
            }
            counted += 1;
            let name = cache.models.get(model).map_or("unknown", String::as_str);
            s.cost.all_time += cost;
            bump(&mut s.by_model, name, tokens, cost);
            if ts >= days[0]
                && let Some(i) = days.iter().rposition(|d| ts >= *d)
                && let Some(day) = s.daily.get_mut(i)
            {
                day.tokens += tokens;
                day.cost += cost;
            }
            if ts >= today {
                counted_today = true;
                s.tokens.today += tokens;
                s.cost.today += cost;
            }
            if ts >= week {
                s.tokens.week += tokens;
                bump(&mut s.by_model_week, name, tokens, cost);
            }
            if ts >= month {
                s.tokens.month += tokens;
                bump(&mut s.by_model_month, name, tokens, cost);
            }
        }
        if counted > 0 {
            s.session_count += 1;
        }
        if counted_today {
            s.sessions_today += 1;
        }
        let dir = std::path::Path::new(path)
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        for &(at, before, after, replies, paid, context) in &file.savings {
            if !seen_compactions.insert((at, before.to_bits())) {
                continue;
            }
            let c = CompactionSaving {
                at,
                before,
                after,
                replies,
                paid,
                context,
            };
            all.push(c.clone());
            by_dir.entry(dir.clone()).or_default().push(c);
        }
    }
    s.compaction = totals_of(&all).into();
    s.compaction_by_dir = by_dir
        .into_iter()
        .map(|(d, l)| (d, totals_of(&l).into()))
        .collect();
    s
}

/// Stamp of a `sessions-list` item: `(path, modified ms, size)`.
#[must_use]
pub fn stamp(info: &Value) -> Option<(String, i64, u64)> {
    let path = info.get("path")?.as_str()?.to_owned();
    let modified = match info.get("modified") {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(s)) => crate::time::parse_iso_ms(s).unwrap_or(0),
        Some(Value::Object(o)) => {
            o.get("secs_since_epoch")
                .and_then(Value::as_i64)
                .unwrap_or(0)
                * 1000
        }
        _ => 0,
    };
    let size = info
        .get("size")
        .or_else(|| info.get("messageCount"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    Some((path, modified, size))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const NOW: i64 = 1_767_261_600_000; // 2026-01-01T10:00:00Z (Thursday)

    fn reply(ts: &str, model: &str, input: u64, cost: f64) -> Value {
        json!({"type": "message", "timestamp": ts, "message": {"role": "assistant", "provider": "p", "model": model,
            "usage": {"input": input, "output": 10, "cacheRead": 100, "cacheWrite": 0, "cost": {"total": cost}}}})
    }

    #[test]
    fn bounds_are_utc_monday_and_first() {
        let (today, week, month, days) = bounds(NOW);
        assert_eq!(today, 1_767_225_600_000);
        assert_eq!(today - week, 3 * DAY_MS, "Thursday → Monday");
        assert_eq!(month, today, "the 1st");
        assert_eq!(days.len(), 30);
        assert_eq!(days[29], today);
        assert_eq!(civil(0), (1970, 1, 1));
    }

    #[test]
    fn extract_and_aggregate_with_dedup() {
        let mut models = Vec::new();
        let a = vec![
            reply("2026-01-01T09:00:00.000Z", "opus", 1000, 0.5),
            reply("2025-12-20T09:00:00.000Z", "haiku", 50, 0.01),
            json!({"type": "custom", "customType": "unipi-child-usage", "timestamp": "2026-01-01T09:30:00.000Z", "data": {"model": "anthropic/sonnet", "input": 10, "output": 5, "cost": 0.2}}),
            json!({"type": "custom", "customType": "unipi-child-usage", "timestamp": "2026-01-01T09:31:00.000Z", "data": {"standalone": false, "input": 10, "cost": 9.0}}),
            json!({"type": "usage", "kind": "child:explore", "provider": "x", "model": "mini", "timestamp": "2026-01-01T09:40:00.000Z", "usage": {"input": 1, "output": 1, "cost": {"total": 0.05}}}),
        ];
        let (ra, sa) = extract(&a, &mut models);
        assert_eq!(ra.len(), 4);
        assert_eq!(sa, [] as [(i64, f64, f64, u64, f64, f64); 0]);
        // A forked copy repeats the first reply: counted once.
        let (rb, _) = extract(&a[..1], &mut models);
        let mut cache = CacheFile {
            version: CACHE_VERSION,
            models: models.clone(),
            files: BTreeMap::new(),
        };
        cache.files.insert(
            "/s/--p--/a.jsonl".into(),
            CachedFile {
                mtime_ms: 1,
                size: 1,
                records: ra,
                savings: vec![],
            },
        );
        cache.files.insert(
            "/s/--p--/b.jsonl".into(),
            CachedFile {
                mtime_ms: 1,
                size: 1,
                records: rb,
                savings: vec![],
            },
        );
        let s = aggregate(&cache, NOW);
        assert!((s.cost.all_time - 0.76).abs() < 1e-9);
        assert!((s.cost.today - 0.75).abs() < 1e-9);
        assert_eq!(s.session_count, 1, "the fork contributed nothing new");
        assert_eq!(s.sessions_today, 1);
        assert!(
            (s.tokens.today - (1010.0 + 15.0 + 2.0)).abs() < 1e-9,
            "cacheRead excluded"
        );
        assert!(s.by_model.contains_key("sonnet") && s.by_model.contains_key("mini"));
        assert!(!s.by_model_month.contains_key("haiku"), "last month");
        assert!((s.daily[29].cost - 0.75).abs() < 1e-9);
        assert!(
            (s.daily[17].cost - 0.01).abs() < 1e-9,
            "Dec 20 is 12 days before Jan 1"
        );
    }

    #[test]
    fn savings_are_deduped_per_compaction() {
        let mut models = Vec::new();
        let e = vec![
            json!({"type": "compaction", "timestamp": "2026-01-01T08:00:00.000Z", "tokensBefore": 10000, "details": {"tokensAfter": 2000}}),
            json!({"type": "message", "message": {"role": "assistant", "usage": {"input": 2000, "cost": {"input": 0.02}}}}),
        ];
        let (_, rows) = extract(&e, &mut models);
        assert_eq!(rows.len(), 1);
        let mut cache = CacheFile {
            version: CACHE_VERSION,
            ..CacheFile::default()
        };
        cache.files.insert(
            "/s/--a--/1.jsonl".into(),
            CachedFile {
                savings: rows.clone(),
                ..CachedFile::default()
            },
        );
        cache.files.insert(
            "/s/--a--/2.jsonl".into(),
            CachedFile {
                savings: rows,
                ..CachedFile::default()
            },
        );
        let s = aggregate(&cache, NOW);
        assert_eq!(s.compaction.count, 1);
        assert!((s.compaction.saved - 8000.0).abs() < 1e-9);
        assert_eq!(s.compaction_by_dir["--a--"].count, 1);
    }

    #[test]
    fn stamps() {
        assert_eq!(
            stamp(&json!({"path": "/a", "modified": 5, "messageCount": 3})),
            Some(("/a".into(), 5, 3))
        );
        assert_eq!(
            stamp(&json!({"path": "/a", "modified": "1970-01-01T00:00:01.000Z", "size": 9})),
            Some(("/a".into(), 1000, 9))
        );
        assert_eq!(stamp(&json!({"modified": 1})), None);
    }
}
