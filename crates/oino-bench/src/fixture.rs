//! Session fixtures (spec §6): deterministic v3 session files of a given entry count, with
//! `compaction_state` checkpoints and abandoned branches, shaped like the owner's real
//! chats (≈ 5 % messages; the rest `custom` subagent steps and `usage` rows). Plus the
//! real 72 MB chat (`~/.cache/perf-fixture/example-api-72mb.jsonl`), copied with its
//! header `cwd` rewritten — that file is never committed.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, TimeZone, Utc};
use serde_json::{Value, json};

/// Entries per turn: user, assistant tool call, tool result, assistant answer + filler.
const FILLER_PER_TURN: usize = 16;
/// A `compaction_state` every this many turns.
const COMPACT_EVERY: usize = 150;
/// An abandoned two-turn branch every this many turns.
const BRANCH_EVERY: usize = 97;

/// A generated or copied session.
#[derive(Clone, Debug)]
pub(crate) struct Fixture {
    pub(crate) session_id: String,
    pub(crate) path: PathBuf,
    pub(crate) entries: usize,
}

/// The pi session-file stem of a timestamp (`2026-10-01T00-00-00-000Z`).
fn stem(ts: &DateTime<Utc>) -> String {
    ts.format("%Y-%m-%dT%H-%M-%S-%3fZ").to_string()
}

fn iso(ts: &DateTime<Utc>) -> String {
    ts.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

/// `--<cwd>--`, as `oino_session::manager::encode_cwd` (absolute `cwd`).
pub(crate) fn session_dir(agent_dir: &Path, cwd: &Path) -> PathBuf {
    agent_dir
        .join("sessions")
        .join(oino_session::manager::encode_cwd(cwd))
}

fn usage() -> Value {
    json!({"input": 1200, "output": 300, "cacheRead": 90000, "cacheWrite": 0, "totalTokens": 91500,
           "cost": {"input": 0.0, "output": 0.0, "cacheRead": 0.0, "cacheWrite": 0.0, "total": 0.0}})
}

struct Writer {
    out: BufWriter<File>,
    prefix: u32,
    next: u64,
    parent: Option<String>,
    /// The current branch, as entry ids (the path `anchorMessage.index` counts into).
    path: Vec<String>,
    written: usize,
    t0: DateTime<Utc>,
}

impl Writer {
    fn put(&mut self, mut entry: Value) -> Result<String> {
        self.next += 1;
        let id = format!("{:03x}{:05x}", self.prefix & 0xfff, self.next);
        let ts =
            self.t0 + ChronoDuration::milliseconds(i64::try_from(self.next).unwrap_or(0) * 250);
        entry["id"] = json!(id);
        entry["parentId"] = self.parent.as_ref().map_or(Value::Null, |p| json!(p));
        entry["timestamp"] = json!(iso(&ts));
        serde_json::to_writer(&mut self.out, &entry)?;
        self.out.write_all(b"\n")?;
        self.parent = Some(id.clone());
        self.path.push(id.clone());
        self.written += 1;
        Ok(id)
    }

    fn message(&mut self, message: &Value) -> Result<String> {
        self.put(json!({"type": "message", "message": message}))
    }

    fn ms(&self) -> i64 {
        self.t0.timestamp_millis() + i64::try_from(self.next).unwrap_or(0) * 250
    }

    /// One turn; returns the user message id.
    fn turn(&mut self, n: usize, filler: usize) -> Result<String> {
        let ms = self.ms();
        let user = self.message(&json!({"role": "user", "timestamp": ms,
            "content": [{"type": "text", "text": format!("Turn {n}: build the next step and run the tests.")}]}))?;
        let call = format!("call_{}_{n}", self.prefix);
        self.message(&json!({"role": "assistant", "timestamp": ms, "api": "openai-completions",
            "provider": "bench", "model": "faux-1", "usage": usage(), "stopReason": "toolUse",
            "content": [{"type": "text", "text": "Running the build."},
                        {"type": "toolCall", "id": call, "name": "bash", "arguments": {"command": format!("cargo test -p crate_{n}")}}]}))?;
        for i in 0..filler {
            if i % 2 == 0 {
                self.put(
                    json!({"type": "usage", "kind": "child:Subagent \"bench\"", "provider": "bench",
                    "model": "faux-1", "usage": usage()}),
                )?;
            } else {
                self.put(
                    json!({"type": "custom", "customType": "subagent-step", "data": {
                    "kind": "tool", "name": "read", "arg": format!("/work/src/file_{n}_{i}.rs"),
                    "output": "fn step() { /* bench filler line */ }\n".repeat(24)}}),
                )?;
            }
        }
        self.message(&json!({"role": "toolResult", "toolCallId": call, "toolName": "bash", "isError": false,
            "timestamp": ms, "content": [{"type": "text", "text": "test result: ok. 42 passed; 0 failed\n".repeat(64)}]}))?;
        self.message(&json!({"role": "assistant", "timestamp": ms, "api": "openai-completions",
            "provider": "bench", "model": "faux-1", "usage": usage(), "stopReason": "stop",
            "content": [{"type": "text", "text": format!("## Turn {n} done\n\nAll tests pass. Next I will wire the remaining step.")}]}))?;
        Ok(user)
    }
}

/// Writes a session with ~`entries` entries for `cwd` under `agent_dir/sessions`.
///
/// # Errors
///
/// I/O failures.
pub(crate) fn generate(agent_dir: &Path, cwd: &Path, entries: usize, seed: u32) -> Result<Fixture> {
    let t0 = Utc
        .with_ymd_and_hms(2026, 10, 1, 0, 0, 0)
        .single()
        .context("fixture epoch")?
        + ChronoDuration::seconds(i64::from(seed));
    let session_id = format!("0b0e0c00-0000-7000-8000-{seed:012x}");
    let dir = session_dir(agent_dir, cwd);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}_{session_id}.jsonl", stem(&t0)));
    let mut w = Writer {
        out: BufWriter::new(File::create(&path)?),
        prefix: seed,
        next: 0,
        parent: None,
        path: Vec::new(),
        written: 0,
        t0,
    };
    serde_json::to_writer(
        &mut w.out,
        &json!({"type": "session", "version": 3, "id": session_id, "timestamp": iso(&t0), "cwd": cwd.to_string_lossy()}),
    )?;
    w.out.write_all(b"\n")?;
    w.put(json!({"type": "model_change", "provider": "bench", "modelId": "faux-1"}))?;
    w.put(json!({"type": "thinking_level_change", "thinkingLevel": "off"}))?;
    let filler = if entries < 500 { 2 } else { FILLER_PER_TURN };
    let mut users: Vec<(String, usize)> = Vec::new();
    let mut turn = 0usize;
    while w.written + 4 + filler <= entries.max(8) {
        if turn > 0 && turn.is_multiple_of(BRANCH_EVERY) && w.written + 3 * (4 + filler) < entries {
            // An abandoned branch: two turns off the current leaf, then the main line
            // continues from the same fork point (the leaf stays on the main line).
            let (fork, path_len) = (w.parent.clone(), w.path.len());
            w.turn(turn, filler)?;
            w.turn(turn + 1, filler)?;
            w.parent = fork;
            w.path.truncate(path_len);
        }
        let index = w.path.len();
        let user = w.turn(turn, filler)?;
        users.push((user, index));
        turn += 1;
        if turn.is_multiple_of(COMPACT_EVERY) && users.len() > 3 {
            let (anchor, index) = users[users.len() - 3].clone();
            w.put(json!({"type": "compaction_state", "summaryText": format!("## Summary up to turn {turn}\n\n{}", "Progress notes. ".repeat(200)),
                "summaryTokens": 900, "summaryKind": "llm_summary", "anchorMessage": {"id": anchor, "index": index},
                "removedCount": turn.saturating_sub(3) * 4, "retainedSkills": [], "retainedImages": [], "loadedDeferredToolNames": []}))?;
        }
    }
    w.put(json!({"type": "session_info", "name": format!("Bench {entries}")}))?;
    w.out.flush()?;
    Ok(Fixture {
        session_id,
        path,
        entries: w.written,
    })
}

/// Copies the real large chat into `agent_dir` for `cwd` (header `cwd` rewritten).
///
/// # Errors
///
/// I/O failures or a file without a session header.
pub(crate) fn copy_real(agent_dir: &Path, cwd: &Path, source: &Path) -> Result<Fixture> {
    let mut lines =
        BufReader::new(File::open(source).with_context(|| format!("open {}", source.display()))?)
            .lines();
    let mut header: Value = serde_json::from_str(&lines.next().context("empty fixture")??)?;
    let session_id = header["id"]
        .as_str()
        .context("fixture header id")?
        .to_owned();
    let ts = header["timestamp"]
        .as_str()
        .unwrap_or("2026-01-01T00:00:00.000Z")
        .replace([':', '.'], "-");
    header["cwd"] = json!(cwd.to_string_lossy());
    let dir = session_dir(agent_dir, cwd);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{ts}_{session_id}.jsonl"));
    let mut out = BufWriter::new(File::create(&path)?);
    serde_json::to_writer(&mut out, &header)?;
    out.write_all(b"\n")?;
    let mut entries = 0;
    for line in lines {
        let line = line?;
        out.write_all(line.as_bytes())?;
        out.write_all(b"\n")?;
        entries += 1;
    }
    out.flush()?;
    Ok(Fixture {
        session_id,
        path,
        entries,
    })
}

/// The real chat's default location.
pub(crate) fn real_source() -> Option<PathBuf> {
    let path =
        PathBuf::from(std::env::var_os("HOME")?).join(".cache/perf-fixture/example-api-72mb.jsonl");
    path.is_file().then_some(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_session::context::build_session_projection;
    use oino_session::manager::SessionManager;

    #[test]
    fn generated_sessions_load_with_valid_compactions_and_branches() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let cwd = dir.path().join("work");
        std::fs::create_dir_all(&cwd)?;
        for (n, seed) in [(100, 1), (5_000, 2)] {
            let f = generate(dir.path(), &cwd, n, seed)?;
            assert!(f.entries <= n && f.entries + 25 >= n, "{n} → {}", f.entries);
            let manager = SessionManager::open(&f.path, &session_dir(dir.path(), &cwd), None)?;
            assert_eq!(manager.session_id(), f.session_id);
            assert_eq!(manager.entries().len(), f.entries);
            let projection = build_session_projection(manager.index(), manager.leaf_id());
            assert!(projection.errors.is_empty(), "{:?}", projection.errors);
            if n == 5_000 {
                // Branched: the current branch is shorter than the file.
                assert!(manager.branch(None).len() < f.entries);
                assert!(
                    manager
                        .entries()
                        .iter()
                        .any(|e| e.type_name() == "compaction_state")
                );
                // The context starts at the last checkpoint, not the whole history.
                let on_branch = manager
                    .branch(None)
                    .iter()
                    .filter(|e| e.type_name() == "message")
                    .count();
                assert!(
                    projection.messages.len() < on_branch / 2,
                    "{} of {on_branch}",
                    projection.messages.len()
                );
            }
        }
        Ok(())
    }
}
