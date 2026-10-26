//! Stale cleanup, allowlist only (unipi `src/lifecycle/cleanup.ts`).
//!
//! Only the targets in [`targets`] can ever be removed; everything else is out of reach by
//! construction. Callers preview first and delete only after the user confirms. Oino scopes
//! the allowlist to the utility's own `/tmp`: other extensions' and
//! core's temp trees are host-managed (D10 state GC).

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use regex::Regex;

/// One allowlisted target.
#[derive(Clone, Debug)]
pub struct Target {
    /// Id.
    pub id: &'static str,
    /// Preview label.
    pub label: &'static str,
    /// Directory scanned (non-recursive).
    pub dir: PathBuf,
    /// Entry names that belong to this target.
    pub pattern: Regex,
    /// Minimum age in days (0 = any age).
    pub min_age_days: u64,
    /// Entry kind.
    pub kind: EntryKind,
}

/// Entry kind to consider.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// Files.
    File,
    /// Directories.
    Dir,
}

/// One removable entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanupItem {
    /// Target id.
    pub target: &'static str,
    /// Guest path.
    pub path: PathBuf,
    /// Size.
    pub bytes: u64,
}

/// Removal outcome.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CleanupResult {
    /// Removed entries.
    pub removed: usize,
    /// Bytes freed.
    pub bytes: u64,
    /// Paths that could not be removed.
    pub failed: Vec<PathBuf>,
}

/// The Oino allowlist.
#[must_use]
pub fn targets() -> Vec<Target> {
    let re = |s: &str| Regex::new(s).ok();
    [
        (
            "tool-results",
            "Saved tool outputs older than 7 days",
            "/tmp/tool-results",
            r"^(?:tool-result|mcp-[a-zA-Z0-9_-]+|helper)-[a-f0-9-]+\.txt$",
            7,
            EntryKind::File,
        ),
        (
            "temp",
            "UniPi temp files older than 7 days",
            "/tmp",
            r"^unipi-",
            7,
            EntryKind::File,
        ),
    ]
    .into_iter()
    .filter_map(|(id, label, dir, pat, min_age_days, kind)| {
        Some(Target {
            id,
            label,
            dir: PathBuf::from(dir),
            pattern: re(pat)?,
            min_age_days,
            kind,
        })
    })
    .collect()
}

fn size_of(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(m) if m.is_dir() => std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| size_of(&e.path()))
            .sum(),
        Ok(m) => m.len(),
        Err(_) => 0,
    }
}

/// Everything the allowlist would remove right now. Never deletes.
#[must_use]
pub fn find(targets: &[Target], now: SystemTime) -> Vec<CleanupItem> {
    let mut items = Vec::new();
    for t in targets {
        let Ok(entries) = std::fs::read_dir(&t.dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !t.pattern.is_match(&name) {
                continue;
            }
            let path = e.path();
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            let kind_ok = match t.kind {
                EntryKind::File => meta.is_file(),
                EntryKind::Dir => meta.is_dir(),
            };
            if !kind_ok {
                continue;
            }
            if t.min_age_days > 0 {
                let age = meta
                    .modified()
                    .ok()
                    .and_then(|m| now.duration_since(m).ok())
                    .unwrap_or_default();
                if age < Duration::from_secs(t.min_age_days * 86_400) {
                    continue;
                }
            }
            items.push(CleanupItem {
                target: t.id,
                bytes: size_of(&path),
                path,
            });
        }
    }
    items
}

/// Deletes exactly the previewed items, re-validated against the allowlist.
#[must_use]
pub fn remove(items: &[CleanupItem], targets: &[Target], now: SystemTime) -> CleanupResult {
    let allowed: Vec<PathBuf> = find(targets, now).into_iter().map(|i| i.path).collect();
    let mut r = CleanupResult::default();
    for item in items {
        if !allowed.contains(&item.path) {
            continue;
        }
        let done = if item.path.is_dir() {
            std::fs::remove_dir_all(&item.path)
        } else {
            std::fs::remove_file(&item.path)
        };
        match done {
            Ok(()) => {
                r.removed += 1;
                r.bytes += item.bytes;
            }
            Err(_) => r.failed.push(item.path.clone()),
        }
    }
    r
}

/// unipi utility `formatBytes` (`B`, one-decimal `KB` / `MB`).
#[must_use]
pub fn format_bytes(bytes: u64) -> String {
    #[allow(clippy::cast_precision_loss, reason = "display only")]
    let f = bytes as f64;
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", f / 1024.0)
    } else {
        format!("{:.1} MB", f / 1024.0 / 1024.0)
    }
}

/// Preview grouped by target (dry run and confirm dialog).
#[must_use]
pub fn preview(
    items: &[CleanupItem],
    targets: &[Target],
    show: &dyn Fn(&Path) -> String,
) -> String {
    if items.is_empty() {
        return "Nothing to clean.".into();
    }
    let total: u64 = items.iter().map(|i| i.bytes).sum();
    let mut lines = vec![format!("{} item(s), {}:", items.len(), format_bytes(total))];
    for t in targets {
        let mine: Vec<&CleanupItem> = items.iter().filter(|i| i.target == t.id).collect();
        if mine.is_empty() {
            continue;
        }
        let bytes: u64 = mine.iter().map(|i| i.bytes).sum();
        lines.push(format!(
            "- {}: {} ({})",
            t.label,
            mine.len(),
            format_bytes(bytes)
        ));
        for i in mine.iter().take(5) {
            lines.push(format!("    {}", show(&i.path)));
        }
        if mine.len() > 5 {
            lines.push(format!("    … and {} more", mine.len() - 5));
        }
    }
    lines.join("\n")
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

    struct Fixture {
        root: PathBuf,
        old: PathBuf,
        fresh: PathBuf,
        foreign: PathBuf,
        legacy: PathBuf,
        keep: PathBuf,
        targets: Vec<Target>,
    }

    fn fixture(name: &str) -> Fixture {
        let base = std::env::var_os("TMPDIR").map_or_else(std::env::temp_dir, PathBuf::from);
        let root = base.join(format!("unipi-cleanup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let results = root.join("tool-results");
        std::fs::create_dir_all(&results).unwrap();
        let old = results.join("tool-result-abc123.txt");
        let fresh = results.join("tool-result-def456.txt");
        let foreign = results.join("memory.db");
        for f in [&old, &fresh, &foreign] {
            std::fs::write(f, "x".repeat(10)).unwrap();
        }
        let legacy = root.join("db/compactor");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("session.db"), "db").unwrap();
        let keep = root.join("memory-v2-backup-1/p/memory.db");
        std::fs::create_dir_all(keep.parent().unwrap()).unwrap();
        std::fs::write(&keep, "keep").unwrap();
        let targets = vec![
            Target {
                id: "tool-results",
                label: "tool results",
                dir: results,
                pattern: Regex::new(r"^tool-result-[a-f0-9-]+\.txt$").unwrap(),
                min_age_days: 7,
                kind: EntryKind::File,
            },
            Target {
                id: "compactor-db",
                label: "legacy db",
                dir: root.join("db"),
                pattern: Regex::new("^compactor$").unwrap(),
                min_age_days: 0,
                kind: EntryKind::Dir,
            },
        ];
        Fixture {
            root,
            old,
            fresh,
            foreign,
            legacy,
            keep,
            targets,
        }
    }

    /// "Now" ten days ahead makes everything the fixture wrote ten days old.
    fn later() -> SystemTime {
        SystemTime::now() + Duration::from_secs(10 * 86_400)
    }

    #[test]
    fn previews_only_allowlisted_old_enough_entries() {
        let f = fixture("preview");
        let now = later();
        let mut paths: Vec<PathBuf> = find(&f.targets, now).into_iter().map(|i| i.path).collect();
        paths.sort();
        let mut want = vec![f.legacy.clone(), f.old.clone(), f.fresh.clone()];
        want.sort();
        assert_eq!(paths, want, "ten days later both tool results are old");
        let young = find(&f.targets, SystemTime::now());
        assert_eq!(
            young.iter().map(|i| i.path.clone()).collect::<Vec<_>>(),
            vec![f.legacy.clone()]
        );
        assert!(
            preview(&find(&f.targets, now), &f.targets, &|p| p
                .to_string_lossy()
                .into_owned())
            .starts_with("3 item(s)")
        );
        assert_eq!(
            preview(&[], &f.targets, &|_| String::new()),
            "Nothing to clean."
        );
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn removes_the_previewed_items_and_nothing_else() {
        let f = fixture("remove");
        let now = SystemTime::now();
        let result = remove(&find(&f.targets, now), &f.targets, now);
        assert_eq!(result.removed, 1);
        assert!(!f.legacy.exists());
        assert!(f.old.exists() && f.fresh.exists() && f.foreign.exists() && f.keep.exists());
        let refused = remove(
            &[CleanupItem {
                target: "tool-results",
                path: f.keep.clone(),
                bytes: 4,
            }],
            &f.targets,
            later(),
        );
        assert_eq!(refused.removed, 0);
        assert!(f.keep.exists());
        let all = remove(&find(&f.targets, later()), &f.targets, later());
        assert_eq!(all.removed, 2);
        assert!(f.foreign.exists());
        let _ = std::fs::remove_dir_all(&f.root);
    }

    #[test]
    fn bytes_and_targets() {
        assert_eq!(format_bytes(10), "10 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(targets().len(), 2);
        assert!(targets()[0].pattern.is_match("mcp-github-0a1b.txt"));
        assert!(!targets()[0].pattern.is_match("notes.txt"));
    }
}
