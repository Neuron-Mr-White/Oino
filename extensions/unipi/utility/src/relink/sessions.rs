//! Session files after a project move (unipi `src/move/pi-state.ts` `piSessionsHandler`).
//!
//! Oino keeps sessions in `~/.oino/sessions/--<cwd>--/` (pi layout). Every `.jsonl` of the old
//! root's directory moves to the new root's directory with the header `cwd` rewritten (the
//! body bytes are preserved); same-name conflicts are skipped, the emptied old directory is
//! removed. Operates on a guest sessions root (`/oino-sessions`), so it is plain `std::fs`.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{Item, LogEntry};
use crate::hostfs::normalize;

/// `--<cwd>--` directory name (oino-session `encode_cwd`).
#[must_use]
pub fn encode_cwd(cwd: &str) -> String {
    let text = normalize(Path::new(cwd)).to_string_lossy().into_owned();
    let trimmed = text.strip_prefix(['/', '\\']).unwrap_or(&text);
    format!("--{}--", trimmed.replace(['/', '\\', ':'], "-"))
}

/// First line (header) and the byte offset of the body.
fn header(bytes: &[u8]) -> Option<(String, usize)> {
    let end = bytes
        .iter()
        .position(|b| *b == b'\n')
        .unwrap_or(bytes.len());
    let line = String::from_utf8(bytes[..end].to_vec()).ok()?;
    let line = line
        .strip_prefix('\u{feff}')
        .map(ToOwned::to_owned)
        .unwrap_or(line);
    serde_json::from_str::<Value>(&line).ok()?;
    Some((line, (end + 1).min(bytes.len())))
}

fn header_value(file: &Path) -> Option<Value> {
    let bytes = std::fs::read(file).ok()?;
    let (line, _) = header(&bytes)?;
    serde_json::from_str(&line).ok()
}

fn jsonl(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| {
            Path::new(n)
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
        })
        .collect();
    v.sort();
    v
}

/// Header `cwd` of the newest session in every directory whose cwd `gone(cwd)` says is gone.
pub fn discover_orphans(root: &Path, gone: &dyn Fn(&str) -> bool) -> Vec<String> {
    let mut out = Vec::new();
    for dir in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let path = dir.path();
        if !path.is_dir() {
            continue;
        }
        let newest = jsonl(&path).into_iter().max_by_key(|n| {
            std::fs::metadata(path.join(n))
                .and_then(|m| m.modified())
                .ok()
        });
        let Some(name) = newest else { continue };
        let cwd = header_value(&path.join(name))
            .filter(|h| h.get("type").and_then(Value::as_str) == Some("session"))
            .and_then(|h| h.get("cwd").and_then(Value::as_str).map(ToOwned::to_owned));
        if let Some(cwd) = cwd
            && !cwd.is_empty()
            && !out.contains(&cwd)
            && gone(&cwd)
        {
            out.push(cwd);
        }
    }
    out
}

/// One session file to move.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionMove {
    /// Guest source.
    pub from: PathBuf,
    /// Guest target.
    pub to: PathBuf,
    /// Old root.
    pub old_root: String,
    /// New root.
    pub new_root: String,
    /// `parentSession` stays at the old path.
    pub lineage: Option<String>,
}

/// Items for `old → new` (`host(guest)` renders paths for the plan).
pub fn scan(
    root: &Path,
    old_root: &str,
    new_root: &str,
    host: &dyn Fn(&Path) -> String,
) -> Vec<Item> {
    let source = root.join(encode_cwd(old_root));
    let target = root.join(encode_cwd(new_root));
    if source == target || !source.is_dir() {
        return Vec::new();
    }
    let source_host = host(&source);
    jsonl(&source)
        .into_iter()
        .map(|name| {
            let from = source.join(&name);
            let to = target.join(&name);
            let conflict = to.exists();
            let lineage = header_value(&from)
                .and_then(|h| {
                    h.get("parentSession")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
                .filter(|p| p.starts_with(&format!("{source_host}/")));
            let notice = if lineage.is_some() {
                "; parentSession lineage remains at the old path (only cwd is rewritten)"
            } else {
                ""
            };
            let description = if conflict {
                format!("Skip conflict: {} → {}", host(&from), host(&to))
            } else {
                format!(
                    "{} → {}; rewrite header cwd {old_root} → {new_root}{notice}",
                    host(&from),
                    host(&to)
                )
            };
            Item::Session {
                description,
                plan: SessionMove {
                    from,
                    to,
                    old_root: old_root.to_owned(),
                    new_root: new_root.to_owned(),
                    lineage,
                },
            }
        })
        .collect()
}

/// Applies one move; returns its log entries.
///
/// # Errors
///
/// I/O failures (the source stays in place).
pub fn apply(m: &SessionMove, host: &dyn Fn(&Path) -> String) -> std::io::Result<Vec<LogEntry>> {
    let (from_h, to_h) = (host(&m.from), host(&m.to));
    let entry = |action: &str, from: &str, to: &str, result: &str| {
        LogEntry::new("pi-sessions", action, from, to, result)
    };
    if m.to.exists() {
        return Ok(vec![entry(
            "move",
            &from_h,
            &to_h,
            "skipped: same-name conflict",
        )]);
    }
    let mut log = Vec::new();
    let bytes = std::fs::read(&m.from)?;
    let target_dir = m.to.parent().map(Path::to_path_buf).unwrap_or_default();
    std::fs::create_dir_all(&target_dir)?;
    log.push(entry("mkdir", "", &host(&target_dir), "ok"));
    let temporary = m.to.with_extension(format!("jsonl.move-{}", oino_now()));
    let out = match header(&bytes) {
        Some((line, offset)) => {
            let mut h: Value = serde_json::from_str(&line).map_err(std::io::Error::other)?;
            if h.get("cwd").and_then(Value::as_str) == Some(m.old_root.as_str()) {
                h["cwd"] = Value::String(m.new_root.clone());
                let mut out = serde_json::to_vec(&h).map_err(std::io::Error::other)?;
                out.push(b'\n');
                out.extend_from_slice(&bytes[offset..]);
                let mut e = entry("rewrite-header", &from_h, &host(&temporary), "ok");
                e.original_header = Some(line);
                log.push(e);
                out
            } else {
                bytes
            }
        }
        None => bytes,
    };
    let written = std::fs::write(&temporary, &out).and_then(|()| {
        log.push(entry("copy", &from_h, &host(&temporary), "ok"));
        if m.to.exists() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "target appeared",
            ));
        }
        std::fs::rename(&temporary, &m.to)
    });
    if let Err(e) = written {
        let _ = std::fs::remove_file(&temporary);
        return Err(e);
    }
    log.push(entry("link", &host(&temporary), &to_h, "ok"));
    std::fs::remove_file(&m.from)?;
    log.push(entry("unlink", &from_h, &to_h, "ok"));
    if let Some(parent) = &m.lineage {
        log.push(entry(
            "parent-session",
            parent,
            &to_h,
            "limitation: fork lineage keeps the original parentSession path; resume still works",
        ));
    }
    if let Some(src) = m.from.parent()
        && std::fs::read_dir(src).is_ok_and(|mut d| d.next().is_none())
        && std::fs::remove_dir(src).is_ok()
    {
        log.push(entry("rmdir", &host(src), &host(&target_dir), "ok"));
    }
    Ok(log)
}

fn oino_now() -> i64 {
    #[cfg(target_arch = "wasm32")]
    {
        oino_extension::timers::now_ms()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(0))
    }
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
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let dir = base.join(format!(
            "unipi-utility-{name}-{}-{}",
            std::process::id(),
            oino_now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn seed(root: &Path, cwd: &str, name: &str) -> (String, &'static str, PathBuf) {
        let dir = root.join(encode_cwd(cwd));
        std::fs::create_dir_all(&dir).unwrap();
        let first =
            serde_json::json!({"type": "session", "version": 3, "id": "move-test", "cwd": cwd})
                .to_string();
        let body = "{\"type\":\"message\",\"content\":\"preserve bytes\\r\\n\"}\r\n";
        let file = dir.join(name);
        std::fs::write(&file, format!("{first}\n{body}")).unwrap();
        (first, body, file)
    }

    fn host(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn encodes_like_oino_session() {
        assert_eq!(
            encode_cwd("/home/u/projects/abc"),
            "--home-u-projects-abc--"
        );
        assert_eq!(encode_cwd("C:\\w\\x"), "--C--w-x--");
        assert_eq!(encode_cwd("/a/./b/../c"), "--a-c--");
    }

    #[test]
    fn move_preserves_body_bytes_logs_header_removes_old_dir_and_is_idempotent() {
        let root = tmp("move");
        let (old, new) = ("/gone/projects/abc", "/home/archived/abc");
        let (first, body, _) = seed(&root, old, "session.jsonl");
        assert_eq!(discover_orphans(&root, &|c| c == old), vec![old.to_owned()]);
        let items = scan(&root, old, new, &host);
        assert_eq!(items.len(), 1);
        let Item::Session { plan, description } = &items[0] else {
            panic!()
        };
        assert!(description.contains("rewrite header cwd"));
        let log = apply(plan, &host).unwrap();
        let moved =
            std::fs::read_to_string(root.join(encode_cwd(new)).join("session.jsonl")).unwrap();
        let (h, rest) = moved.split_once('\n').unwrap();
        assert_eq!(serde_json::from_str::<Value>(h).unwrap()["cwd"], new);
        assert_eq!(rest, body);
        assert_eq!(
            log.iter()
                .find(|e| e.action == "rewrite-header")
                .and_then(|e| e.original_header.clone()),
            Some(first)
        );
        assert!(!root.join(encode_cwd(old)).exists());
        assert!(scan(&root, old, new, &host).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn same_name_conflict_is_skipped_without_overwriting() {
        let root = tmp("conflict");
        let (old, new) = ("/gone/a", "/here/a");
        let (_, _, src) = seed(&root, old, "s.jsonl");
        let (_, _, target) = seed(&root, new, "s.jsonl");
        let before = std::fs::read(&target).unwrap();
        let items = scan(&root, old, new, &host);
        let Item::Session { plan, description } = &items[0] else {
            panic!()
        };
        assert!(description.starts_with("Skip conflict"));
        let log = apply(plan, &host).unwrap();
        assert!(log[0].result.contains("conflict"));
        assert_eq!(std::fs::read(&target).unwrap(), before);
        assert!(src.exists());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn lineage_notice_and_foreign_cwd_headers() {
        let root = tmp("lineage");
        let (old, new) = ("/gone/l", "/here/l");
        let dir = root.join(encode_cwd(old));
        std::fs::create_dir_all(&dir).unwrap();
        let parent = format!("{}/p.jsonl", dir.to_string_lossy());
        std::fs::write(
            dir.join("c.jsonl"),
            format!(
                "{}\n",
                serde_json::json!({"type": "session", "cwd": "/other", "parentSession": parent})
            ),
        )
        .unwrap();
        let items = scan(&root, old, new, &host);
        let Item::Session { plan, description } = &items[0] else {
            panic!()
        };
        assert!(description.contains("parentSession lineage"));
        let log = apply(plan, &host).unwrap();
        assert!(
            log.iter().all(|e| e.action != "rewrite-header"),
            "a foreign cwd is copied unchanged"
        );
        assert!(log.iter().any(|e| e.action == "parent-session"));
        assert!(scan(&root, old, old, &host).is_empty());
        assert!(discover_orphans(&root, &|_| false).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }
}
