//! `usage_stats` (owner fix 2026-11-04, Settings › Usage): token / cost statistics
//! over the session files under the host's sessions root, for the last N local days.
//!
//! Counted records are the assistant replies that carry `usage` (plus oino's `usage`
//! entries and unipi's `unipi-child-usage` custom entries, the spend of child runs).
//! Forks copy history, so records dedupe across files by `(timestamp, tokens)`.
//! Each file is parsed once per `(mtime, size)` and kept in a process-wide cache, so a
//! repeat call only re-reads what changed; files untouched for longer than the range
//! are not opened at all.
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::UNIX_EPOCH;

use serde_json::Value;
use unipi_proto::usage::{
    UsageData, UsageDay, UsageModel, UsageProject, UsageTotals, USAGE_DAILY_MODELS, USAGE_DAYS_MAX, USAGE_OTHER_MODEL,
    USAGE_ROWS_MAX,
};

const DAY_MS: i64 = 86_400_000;

/// One counted reply.
#[derive(Debug, Clone, PartialEq)]
struct Rec {
    ts: i64,
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
    cost: f64,
    model: String,
}

#[derive(Debug, Clone, Default)]
struct Parsed {
    cwd: String,
    recs: Vec<Rec>,
}

struct Cached {
    mtime: i64,
    size: u64,
    parsed: Parsed,
}

fn cache() -> &'static Mutex<HashMap<PathBuf, Cached>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Cached>>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

fn num(v: Option<&Value>) -> f64 {
    v.and_then(Value::as_f64).filter(|n| n.is_finite() && *n > 0.0).unwrap_or(0.0)
}

/// `2026-09-20T16:53:21.276Z` → epoch ms (UTC); `None` when malformed.
fn parse_iso_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let n = |r: std::ops::Range<usize>| s.get(r)?.parse::<i64>().ok();
    let (y, m, d, h, mi, se) = (n(0..4)?, n(5..7)?, n(8..10)?, n(11..13)?, n(14..16)?, n(17..19)?);
    let ms = if b.get(19) == Some(&b'.') { s.get(20..23).and_then(|f| f.parse::<i64>().ok()).unwrap_or(0) } else { 0 };
    Some(days_from_civil(y, m, d) * DAY_MS + ((h * 60 + mi) * 60 + se) * 1000 + ms)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

fn civil(z: i64) -> (i64, i64, i64) {
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

fn day_label(day_index: i64) -> String {
    let (y, m, d) = civil(day_index);
    format!("{y:04}-{m:02}-{d:02}")
}

/// A model id without its provider prefix (`zai/glm-5.3` → `glm-5.3`).
fn model_name(m: &str) -> String {
    m.rsplit_once('/').map_or(m, |(_, id)| id).to_owned()
}

fn record(ts: i64, u: &Value, model: &str, cost: f64) -> Option<Rec> {
    let r = Rec {
        ts,
        input: num(u.get("input")),
        output: num(u.get("output")),
        cache_read: num(u.get("cacheRead")),
        cache_write: num(u.get("cacheWrite")),
        cost,
        model: model_name(model),
    };
    (r.input > 0.0 || r.output > 0.0 || r.cost > 0.0).then_some(r)
}

/// The counted replies of one session file's text. Pure.
fn parse_text(text: &str) -> Parsed {
    let mut out = Parsed::default();
    for line in text.lines() {
        if out.cwd.is_empty() && line.starts_with("{\"type\":\"session\"") {
            if let Ok(v) = serde_json::from_str::<Value>(line) {
                out.cwd = v.get("cwd").and_then(Value::as_str).unwrap_or_default().to_owned();
            }
            continue;
        }
        if !line.contains("\"usage\"") && !line.contains("unipi-child-usage") {
            continue;
        }
        let Ok(e) = serde_json::from_str::<Value>(line) else { continue };
        let entry_ts = e.get("timestamp").and_then(Value::as_str).and_then(parse_iso_ms).unwrap_or(0);
        let rec = match e.get("type").and_then(Value::as_str) {
            Some("message") => {
                let Some(m) = e.get("message").filter(|m| m.get("role").and_then(Value::as_str) == Some("assistant")) else { continue };
                let (Some(u), Some(model)) = (m.get("usage"), m.get("model").and_then(Value::as_str)) else { continue };
                let ts = m.get("timestamp").and_then(Value::as_i64).filter(|t| *t > 0).unwrap_or(entry_ts);
                record(ts, u, model, num(u.pointer("/cost/total")))
            }
            Some("usage") => {
                let u = e.get("usage").unwrap_or(&Value::Null);
                let cost = u.get("cost").map_or(0.0, |c| c.as_f64().map_or_else(|| num(c.get("total")), |n| n.max(0.0)));
                record(entry_ts, u, e.get("model").and_then(Value::as_str).unwrap_or("child-agent"), cost)
            }
            Some("custom") if e.get("customType").and_then(Value::as_str) == Some("unipi-child-usage") => {
                let d = e.get("data").unwrap_or(&Value::Null);
                if d.get("standalone").and_then(Value::as_bool) == Some(false) {
                    continue;
                }
                let cost = d.get("cost").map_or(0.0, |c| c.as_f64().map_or_else(|| num(c.get("total")), |n| n.max(0.0)));
                record(entry_ts, d, d.get("model").and_then(Value::as_str).unwrap_or("child-agent"), cost)
            }
            _ => None,
        };
        out.recs.extend(rec);
    }
    out
}

/// Every `.jsonl` under `dir` with `(mtime ms, size)`.
fn walk(dir: &Path, out: &mut Vec<(PathBuf, i64, u64)>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for entry in rd.flatten() {
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if kind.is_dir() {
            walk(&path, out);
        } else if kind.is_file() && path.extension().is_some_and(|x| x == "jsonl") {
            let Ok(meta) = entry.metadata() else { continue };
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
            out.push((path, mtime, meta.len()));
        }
    }
}

/// The project label of a session file: its recorded folder, else the folder name.
fn project_of(path: &Path, cwd: &str) -> String {
    if !cwd.is_empty() {
        return cwd.to_owned();
    }
    path.ancestors()
        .skip(1)
        .find_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()).filter(|n| n.starts_with("--")))
        .map_or_else(|| "unknown".to_owned(), |n| n.trim_matches('-').to_owned())
}

#[derive(Default)]
struct Bucket {
    tokens: f64,
    cost: f64,
    unpriced: f64,
    count: u64,
}

fn unpriced_of(r: &Rec, tokens: f64) -> f64 {
    if r.cost > 0.0 { 0.0 } else { tokens }
}

/// Aggregates the parsed files at `now_ms` over the last `days` local days. Pure.
fn aggregate(files: &[(PathBuf, Parsed)], now_ms: i64, days: u32, tz_offset_min: i32) -> UsageData {
    let days = days.clamp(1, USAGE_DAYS_MAX);
    let off = i64::from(tz_offset_min) * 60_000;
    let today = (now_ms + off).div_euclid(DAY_MS);
    let first = today - i64::from(days) + 1;
    let mut daily: Vec<UsageDay> = (first..=today).map(|d| UsageDay { day: day_label(d), ..UsageDay::default() }).collect();
    let mut totals = UsageTotals::default();
    let mut by_model: BTreeMap<String, UsageModel> = BTreeMap::new();
    let mut by_project: BTreeMap<String, Bucket> = BTreeMap::new();
    let mut seen: HashSet<(i64, u64)> = HashSet::new();
    let mut active: HashSet<i64> = HashSet::new();
    let mut day_models: Vec<BTreeMap<String, f64>> = vec![BTreeMap::new(); daily.len()];
    for (path, file) in files {
        let mut counted = 0u64;
        let (mut first_ts, mut last_ts) = (i64::MAX, i64::MIN);
        let project = project_of(path, &file.cwd);
        for r in &file.recs {
            let day = (r.ts + off).div_euclid(DAY_MS);
            if day < first || day > today {
                continue;
            }
            let tokens = r.input + r.output + r.cache_read + r.cache_write;
            if !seen.insert((r.ts, tokens.to_bits())) {
                continue;
            }
            counted += 1;
            first_ts = first_ts.min(r.ts);
            last_ts = last_ts.max(r.ts);
            let unpriced = unpriced_of(r, tokens);
            totals.input += r.input;
            totals.output += r.output;
            totals.cache_read += r.cache_read;
            totals.cache_write += r.cache_write;
            totals.tokens += tokens;
            totals.cost += r.cost;
            totals.unpriced_tokens += unpriced;
            totals.turns += 1;
            active.insert(day);
            if let Some(i) = usize::try_from(day - first).ok().filter(|i| *i < daily.len()) {
                let d = &mut daily[i];
                d.tokens += tokens;
                d.cost += r.cost;
                d.unpriced_tokens += unpriced;
                d.turns += 1;
                *day_models[i].entry(r.model.clone()).or_default() += tokens;
            }
            let m = by_model.entry(r.model.clone()).or_insert_with(|| UsageModel { name: r.model.clone(), ..UsageModel::default() });
            m.tokens += tokens;
            m.cost += r.cost;
            m.unpriced_tokens += unpriced;
            m.turns += 1;
            let p = by_project.entry(project.clone()).or_default();
            p.tokens += tokens;
            p.cost += r.cost;
            p.unpriced += unpriced;
        }
        if counted > 0 {
            totals.longest_session_ms = totals.longest_session_ms.max(last_ts - first_ts);
            totals.sessions += 1;
            by_project.entry(project).or_default().count += 1;
        }
    }
    totals.active_days = u32::try_from(active.len()).unwrap_or(u32::MAX);
    let mut models: Vec<UsageModel> = by_model.into_values().collect();
    models.sort_by(|a, b| b.tokens.total_cmp(&a.tokens).then_with(|| a.name.cmp(&b.name)));
    let top: HashSet<&str> = models.iter().take(USAGE_DAILY_MODELS).map(|m| m.name.as_str()).collect();
    for (d, per) in daily.iter_mut().zip(day_models) {
        for (name, t) in per {
            let key = if top.contains(name.as_str()) { name } else { USAGE_OTHER_MODEL.to_owned() };
            *d.models.entry(key).or_default() += t;
        }
    }
    models.truncate(USAGE_ROWS_MAX);
    let mut projects: Vec<UsageProject> = by_project
        .into_iter()
        .map(|(name, b)| UsageProject { name, tokens: b.tokens, cost: b.cost, unpriced_tokens: b.unpriced, sessions: b.count })
        .collect();
    projects.sort_by(|a, b| b.tokens.total_cmp(&a.tokens).then_with(|| a.name.cmp(&b.name)));
    projects.truncate(USAGE_ROWS_MAX);
    UsageData { days, totals, daily, by_model: models, by_project: projects }
}

/// Reads (or takes from the cache) every session file touched inside the range.
fn load(root: &Path, now_ms: i64, days: u32) -> Vec<(PathBuf, Parsed)> {
    let mut found = Vec::new();
    walk(root, &mut found);
    found.sort_by(|a, b| a.0.cmp(&b.0));
    // One day of slack for the timezone; a file untouched since cannot hold a newer reply.
    let cutoff = now_ms - (i64::from(days.clamp(1, USAGE_DAYS_MAX)) + 1) * DAY_MS;
    let mut out = Vec::new();
    let mut cache = cache().lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    for (path, mtime, size) in found {
        if mtime < cutoff {
            continue;
        }
        if let Some(hit) = cache.get(&path).filter(|c| c.mtime == mtime && c.size == size) {
            out.push((path, hit.parsed.clone()));
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else { continue };
        let parsed = parse_text(&text);
        cache.insert(path.clone(), Cached { mtime, size, parsed: parsed.clone() });
        out.push((path, parsed));
    }
    out
}

/// `usage_stats`: blocking (reads files); call from `spawn_blocking`.
#[must_use]
pub fn usage_stats(root: &Path, days: u32, tz_offset_min: i32) -> UsageData {
    let now = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX));
    aggregate(&load(root, now, days), now, days, tz_offset_min)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assistant(ts_iso: &str, ms: i64, model: &str, input: u64, output: u64, cost: f64) -> String {
        format!(
            r#"{{"type":"message","id":"x","timestamp":"{ts_iso}","message":{{"role":"assistant","content":[],"provider":"p","model":"{model}","usage":{{"input":{input},"output":{output},"cacheRead":0,"cacheWrite":0,"cost":{{"total":{cost}}}}},"timestamp":{ms}}}}}"#
        )
    }

    const NOW: i64 = 1_790_000_000_000; // 2026-09-21T14:13:20Z

    #[test]
    fn parses_replies_and_header() {
        let text = format!(
            "{{\"type\":\"session\",\"id\":\"s\",\"cwd\":\"/home/u/app\"}}\n{}\n{{\"type\":\"message\",\"message\":{{\"role\":\"user\"}}}}\n{}\n",
            assistant("2026-09-21T10:00:00.000Z", NOW - 3_600_000, "zai/glm-5", 100, 20, 0.5),
            assistant("2026-09-21T11:00:00.000Z", NOW - 1_000, "free", 10, 5, 0.0),
        );
        let p = parse_text(&text);
        assert_eq!(p.cwd, "/home/u/app");
        assert_eq!(p.recs.len(), 2);
        assert_eq!(p.recs[0].model, "glm-5");
    }

    #[test]
    fn unpriced_models_never_read_as_free_and_forks_dedupe() {
        let a = parse_text(&format!(
            "{{\"type\":\"session\",\"cwd\":\"/p/a\"}}\n{}\n{}\n",
            assistant("2026-09-21T10:00:00.000Z", NOW - 5000, "glm", 100, 20, 0.5),
            assistant("2026-09-21T11:00:00.000Z", NOW - 4000, "free", 10, 5, 0.0),
        ));
        // a fork repeats the first reply and adds one on another day
        let b = parse_text(&format!(
            "{{\"type\":\"session\",\"cwd\":\"/p/a\"}}\n{}\n{}\n",
            assistant("2026-09-21T10:00:00.000Z", NOW - 5000, "glm", 100, 20, 0.5),
            assistant("2026-09-20T11:00:00.000Z", NOW - DAY_MS, "glm", 30, 0, 0.1),
        ));
        let d = aggregate(&[("/s/a.jsonl".into(), a), ("/s/b.jsonl".into(), b)], NOW, 7, 0);
        assert_eq!(d.totals.turns, 3);
        assert_eq!(d.totals.sessions, 2);
        assert_eq!(d.totals.active_days, 2);
        assert!((d.totals.cost - 0.6).abs() < 1e-9);
        assert!((d.totals.unpriced_tokens - 15.0).abs() < 1e-9);
        assert_eq!(d.daily.len(), 7);
        assert_eq!(d.daily[6].day, "2026-09-21");
        assert!((d.daily[6].unpriced_tokens - 15.0).abs() < 1e-9);
        let free = d.by_model.iter().find(|m| m.name == "free").unwrap();
        assert_eq!((free.cost, free.unpriced_tokens), (0.0, 15.0));
        assert_eq!(d.by_project.len(), 1);
        assert_eq!(d.by_project[0].sessions, 2);
        // per-day turns + model split, longest session
        assert_eq!(d.daily[6].turns, 2);
        assert!((d.daily[6].models["glm"] - 120.0).abs() < 1e-9);
        assert!((d.daily[6].models["free"] - 15.0).abs() < 1e-9);
        assert_eq!(d.totals.longest_session_ms, 1000);
    }

    #[test]
    fn range_and_timezone_bound_the_days() {
        let old = parse_text(&assistant("2026-08-01T10:00:00.000Z", NOW - 50 * DAY_MS, "m", 1, 1, 0.1));
        let d = aggregate(&[("/s/o.jsonl".into(), old.clone())], NOW, 7, 0);
        assert_eq!(d.totals.turns, 0);
        assert_eq!(aggregate(&[("/s/o.jsonl".into(), old)], NOW, 90, 0).totals.turns, 1);
        // 14:13Z at UTC+10 is already the next local day
        let e = aggregate(&[], NOW, 1, 600);
        assert_eq!(e.daily[0].day, "2026-09-22");
    }

    #[test]
    fn reads_a_tree_and_reuses_the_cache() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("--home-u-app--");
        std::fs::create_dir_all(&dir).unwrap();
        let now = std::time::SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as i64;
        std::fs::write(dir.join("a.jsonl"), assistant("2026-09-21T10:00:00.000Z", now - 1000, "m", 10, 10, 0.2)).unwrap();
        let d = usage_stats(tmp.path(), 7, 0);
        assert_eq!(d.totals.turns, 1);
        assert_eq!(usage_stats(tmp.path(), 7, 0), d);
        assert_eq!(d.by_project[0].name, "home-u-app");
    }
}
