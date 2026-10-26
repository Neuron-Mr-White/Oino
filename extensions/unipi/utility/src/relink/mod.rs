//! `/unipi:move [<old-path>] [--dry-run]` (unipi `src/move/index.ts`): re-links state after
//! the project folder moved. Never moves the project itself.
//!
//! | unipi move handler | Oino |
//! |---|---|
//! | `pi-sessions` | [`sessions`] over the `/oino-sessions` grant (`~/.oino/sessions`) |
//! | `pi-trust` (`trust.json` key) | host-owned: reported as a manual step (request R2) |
//! | `git-worktrees` | [`git`] (`git worktree repair`) |
//! | registered handlers (memory, kanboard, …) | service fan-out `move.handle{oldRoot,newRoot,dryRun,backup}` |
//!
//! Log: `/logs/move-<ts>.log` (JSON lines). The orphan picker lists session directories whose
//! recorded cwd no longer exists.

pub mod git;
pub mod sessions;

use std::path::{Path, PathBuf};
use std::time::Duration;

use oino_extension::prelude::*;
use serde::Serialize;
use unipi_kit::services::{MoveReport, MoveRequest, fanout as fan};

use crate::hostfs::{self, Roots};

/// Command name.
pub const COMMAND: &str = "unipi:move";
/// Grant of `~/.oino/sessions` (write).
pub const SESSIONS_GRANT: &str = "/oino-sessions";
/// Read-only fallback (`fs.agentDir`).
pub const SESSIONS_READ_ONLY: &str = "/agent/sessions";
const USAGE: &str = "Usage: /unipi:move [<old-path>] [--dry-run]";

/// One log line (unipi `MoveLogEntry`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    /// Handler area.
    pub area: String,
    /// What happened.
    pub action: String,
    /// Source.
    pub from: String,
    /// Target.
    pub to: String,
    /// `ok` / reason.
    pub result: String,
    /// Header before the rewrite.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_header: Option<String>,
}

impl LogEntry {
    /// A new entry.
    #[must_use]
    pub fn new(area: &str, action: &str, from: &str, to: &str, result: &str) -> Self {
        Self {
            area: area.into(),
            action: action.into(),
            from: from.into(),
            to: to.into(),
            result: result.into(),
            original_header: None,
        }
    }
}

/// One planned change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// A session file.
    Session {
        /// Plan line.
        description: String,
        /// The move.
        plan: sessions::SessionMove,
    },
    /// `git worktree repair`.
    GitRepair {
        /// Plan line.
        description: String,
    },
    /// Another extension's handler.
    Handler {
        /// Extension id.
        extension: String,
        /// Its dry-run lines.
        lines: Vec<String>,
    },
    /// A step the user does (host-owned state).
    Manual {
        /// Area.
        area: String,
        /// What to do.
        description: String,
    },
}

impl Item {
    /// Grouping area.
    #[must_use]
    pub fn area(&self) -> String {
        match self {
            Self::Session { .. } => "pi-sessions".into(),
            Self::GitRepair { .. } => "git-worktrees".into(),
            Self::Handler { extension, .. } => extension.clone(),
            Self::Manual { area, .. } => area.clone(),
        }
    }

    /// Plan lines.
    #[must_use]
    pub fn lines(&self) -> Vec<String> {
        match self {
            Self::Session { description, .. }
            | Self::GitRepair { description }
            | Self::Manual { description, .. } => vec![description.clone()],
            Self::Handler { lines, .. } => lines.clone(),
        }
    }
}

/// Parsed arguments: `(old path, dry run)`.
///
/// # Errors
///
/// Unknown options or more than one path.
pub fn parse_args(args: &str) -> Result<(Option<String>, bool), String> {
    let mut paths = Vec::new();
    let mut dry = false;
    for token in tokens(args) {
        if token == "--dry-run" {
            dry = true;
        } else if token.starts_with("--") {
            return Err(format!("Unknown option: {token}"));
        } else {
            paths.push(token);
        }
    }
    if paths.len() > 1 {
        return Err(USAGE.into());
    }
    Ok((paths.pop(), dry))
}

/// Shell-ish tokens: `"…"` (JSON escapes), `'…'`, bare words.
fn tokens(s: &str) -> Vec<String> {
    let c: Vec<char> = s.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        if c[i].is_whitespace() {
            i += 1;
            continue;
        }
        let start = i;
        if c[i] == '"' {
            i += 1;
            while i < c.len() && c[i] != '"' {
                i += if c[i] == '\\' { 2 } else { 1 };
            }
            let raw: String = c[start..(i + 1).min(c.len())].iter().collect();
            out.push(
                serde_json::from_str::<String>(&raw)
                    .unwrap_or_else(|_| raw.trim_matches('"').to_owned()),
            );
            i += 1;
        } else if c[i] == '\'' {
            i += 1;
            while i < c.len() && c[i] != '\'' {
                i += 1;
            }
            out.push(c[start + 1..i.min(c.len())].iter().collect());
            i += 1;
        } else {
            while i < c.len() && !c[i].is_whitespace() {
                i += 1;
            }
            out.push(c[start..i].iter().collect());
        }
    }
    out
}

/// unipi `validateOldRoot` (`exists` = `None` when unknown).
///
/// # Errors
///
/// Same root, or the old path still exists (a copy, not a move).
pub fn validate(old: &str, new: &str, exists: Option<bool>) -> Result<(), String> {
    if old == new {
        return Err("Old path and current project root are the same.".into());
    }
    if exists == Some(true) {
        return Err(format!(
            "Old path still exists: {old}. This is a copy, not a move; refusing to re-link state."
        ));
    }
    Ok(())
}

/// Orphan candidates: same folder name as the cwd first, then alphabetical.
#[must_use]
pub fn sort_candidates(mut roots: Vec<String>, cwd: &str) -> Vec<String> {
    let base = |p: &str| {
        Path::new(p)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let me = base(cwd);
    roots.sort_by(|a, b| (base(b) == me).cmp(&(base(a) == me)).then_with(|| a.cmp(b)));
    roots.dedup();
    roots
}

/// The plan text.
#[must_use]
pub fn plan_text(old: &str, new: &str, items: &[Item], notices: &[String]) -> String {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for item in items {
        let area = item.area();
        match groups.iter_mut().find(|(a, _)| *a == area) {
            Some((_, l)) => l.extend(item.lines()),
            None => groups.push((area, item.lines())),
        }
    }
    let mut parts = vec![format!("Re-link {old} → {new}")];
    for (area, lines) in groups {
        let body: Vec<String> = lines
            .iter()
            .map(|d| format!("  • {}", d.trim_start_matches("  • ")))
            .collect();
        parts.push(format!("\n{area}\n{}", body.join("\n")));
    }
    parts.extend(notices.iter().cloned());
    parts.join("\n")
}

/// `test -e <path>` on the host (`None` when the check is unavailable).
pub async fn exists(path: &str) -> Option<bool> {
    let r = process::exec("test", &["-e", path], None, Some(5_000))
        .await
        .ok()?;
    Some(r.code == Some(0))
}

fn sessions_root() -> Option<PathBuf> {
    [SESSIONS_GRANT, SESSIONS_READ_ONLY]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_dir())
}

/// The command.
pub async fn run(args: &str, cx: &CommandCtx) -> Result<()> {
    if let Err(e) = run_inner(args, cx).await {
        ui::notify(&e, "error")?;
    }
    Ok(())
}

/// Host display of a guest path.
fn host_of(roots: &Roots, p: &Path) -> String {
    roots.to_host(p).map_or_else(
        || p.to_string_lossy().into_owned(),
        |h| h.to_string_lossy().into_owned(),
    )
}

/// Picks the old root: the argument, else the orphan picker (`None` = nothing to do).
async fn choose_old_root(
    old_arg: Option<String>,
    new_root: &str,
    root: Option<&Path>,
    cx: &CommandCtx,
) -> std::result::Result<Option<String>, String> {
    let cwd = cx.cx.cwd().to_string_lossy().into_owned();
    if let Some(o) = old_arg {
        return Ok(Some(hostfs::absolute(&o, &cwd, hostfs::home().as_deref())));
    }
    let mut found = Vec::new();
    for c in root
        .map(|r| sessions::discover_orphans(r, &|_| true))
        .unwrap_or_default()
    {
        if c != new_root && exists(&c).await == Some(false) {
            found.push(c);
        }
    }
    let candidates = sort_candidates(found, &cwd);
    if candidates.is_empty() {
        let _ = ui::notify(
            "No orphaned project state found. Nothing to re-link.",
            "info",
        );
        return Ok(None);
    }
    if !cx.has_ui() {
        return Err(
            "Choosing an old project path requires an interactive UI; pass <old-path>.".into(),
        );
    }
    let opts: Vec<&str> = candidates.iter().map(String::as_str).collect();
    ui::select("Re-link state from which old project path?", &opts)
        .await
        .map_err(|e| e.to_string())
}

/// What a move would do.
struct Plan {
    items: Vec<Item>,
    errors: Vec<String>,
    notices: Vec<String>,
    manual: Item,
}

async fn scan_all(
    old_root: &str,
    new_root: &str,
    root: Option<&Path>,
    roots: &Roots,
    old_exists: Option<bool>,
) -> Plan {
    let host = |p: &Path| host_of(roots, p);
    let mut items: Vec<Item> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    match root {
        Some(r) => items.extend(sessions::scan(r, old_root, new_root, &host)),
        None => errors.push(
            "Pi sessions: the sessions folder is not reachable; sessions were not scanned.".into(),
        ),
    }
    if let Some(d) = git::scan(old_root, new_root).await {
        items.push(Item::GitRepair { description: d });
    }
    let req = MoveRequest {
        old_root: old_root.to_owned(),
        new_root: new_root.to_owned(),
        dry_run: true,
        backup: true,
    };
    for reply in unipi_kit::fanout::call_all(
        fan::MOVE_HANDLE,
        &json!(req),
        Duration::from_secs(10),
        Some(crate::EXTENSION_ID),
    )
    .await
    {
        let report: MoveReport = unipi_kit::services::decode(reply.value).unwrap_or_default();
        if !report.log.is_empty() {
            items.push(Item::Handler {
                extension: reply.extension,
                lines: report.log,
            });
        }
    }
    let mut notices = Vec::new();
    if old_exists.is_none() {
        notices.push(
            "Could not check whether the old path still exists (process checks unavailable)."
                .to_owned(),
        );
    }
    for f in ["mise.toml", ".mise.toml"] {
        if exists(&format!("{new_root}/{f}")).await == Some(true) {
            notices.push(format!(
                "{f}: run mise trust in {new_root} (not changed automatically)."
            ));
        }
    }
    let manual = Item::Manual {
        area: "pi-trust".into(),
        description: format!(
            "trust.json is kept by Oino: if {old_root} was trusted, accept the trust prompt for {new_root} on the next start."
        ),
    };
    Plan {
        items,
        errors,
        notices,
        manual,
    }
}

async fn apply_item(
    item: &Item,
    old_root: &str,
    new_root: &str,
    roots: &Roots,
) -> std::result::Result<Vec<LogEntry>, String> {
    match item {
        Item::Session { plan, .. } => {
            sessions::apply(plan, &|p| host_of(roots, p)).map_err(|e| e.to_string())
        }
        Item::GitRepair { description } => git::repair(new_root)
            .await
            .map(|()| {
                vec![LogEntry::new(
                    "git-worktrees",
                    "repair",
                    old_root,
                    new_root,
                    "ok",
                )]
            })
            .map_err(|e| format!("{description}: {e}")),
        Item::Handler { extension, .. } => {
            let req = MoveRequest {
                old_root: old_root.to_owned(),
                new_root: new_root.to_owned(),
                dry_run: false,
                backup: true,
            };
            oino_extension::services::call(
                Some(extension),
                fan::MOVE_HANDLE,
                &json!(req),
                Some(Duration::from_secs(60)),
            )
            .await
            .map(|v| {
                let report: MoveReport = unipi_kit::services::decode(v).unwrap_or_default();
                report
                    .log
                    .iter()
                    .map(|l| LogEntry::new(extension, "handler", old_root, new_root, l))
                    .collect()
            })
            .map_err(|e| e.to_string())
        }
        Item::Manual { .. } => Ok(Vec::new()),
    }
}

async fn apply_all(plan: &Plan, old_root: &str, new_root: &str, roots: &Roots) {
    let stamp = unipi_kit::fmt::iso_from_ms(timers::now_ms()).replace([':', '.'], "-");
    let log_file = format!("/logs/move-{stamp}.log");
    let mut entries: Vec<LogEntry> = Vec::new();
    let (mut applied, mut failed) = (0usize, 0usize);
    for item in &plan.items {
        match apply_item(item, old_root, new_root, roots).await {
            Ok(log) => {
                applied += 1;
                entries.extend(log);
            }
            Err(e) => {
                failed += 1;
                entries.push(LogEntry::new(
                    &item.area(),
                    &item.lines().join("; "),
                    old_root,
                    new_root,
                    &format!("failed: {e}"),
                ));
            }
        }
    }
    let lines: String = entries
        .iter()
        .filter_map(|e| serde_json::to_string(e).ok())
        .map(|l| l + "\n")
        .collect();
    let _ = std::fs::create_dir_all("/logs");
    let _ = std::fs::write(&log_file, lines);
    let reports: Vec<String> = entries
        .iter()
        .filter(|e| e.result != "ok" && e.action != "handler")
        .map(|e| format!("{}: {}", e.area, e.result))
        .chain(plan.errors.iter().cloned())
        .chain(plan.notices.iter().cloned())
        .chain(plan.manual.lines())
        .collect();
    let warn = failed > 0 || !plan.errors.is_empty();
    let _ = ui::notify(
        &format!(
            "Re-link complete: {applied} item(s) processed, {failed} failed.\n{}\nLog: {}",
            reports.join("\n"),
            host_of(roots, Path::new(&log_file))
        ),
        if warn { "warning" } else { "info" },
    );
}

async fn run_inner(args: &str, cx: &CommandCtx) -> std::result::Result<(), String> {
    let (old_arg, dry) = parse_args(args)?;
    let new_root = host::workspace().root;
    let roots = Roots::current();
    let root = sessions_root();
    let Some(old_root) = choose_old_root(old_arg, &new_root, root.as_deref(), cx).await? else {
        return Ok(());
    };
    let old_exists = exists(&old_root).await;
    validate(&old_root, &new_root, old_exists)?;
    let plan = scan_all(&old_root, &new_root, root.as_deref(), &roots, old_exists).await;
    let mut footer: Vec<String> = plan.notices.iter().chain(&plan.errors).cloned().collect();
    footer.extend(plan.manual.lines());
    let text = plan_text(&old_root, &new_root, &plan.items, &footer);
    let _ = ui::notify(
        &text,
        if plan.errors.is_empty() {
            "info"
        } else {
            "warning"
        },
    );
    let count: usize = plan.items.iter().map(|i| i.lines().len()).sum();
    if dry {
        let _ = ui::notify(
            &format!("Dry run: {count} item(s); no files written."),
            "info",
        );
        return Ok(());
    }
    if plan.items.is_empty() {
        let _ = ui::notify("Nothing to re-link.", "info");
        return Ok(());
    }
    if !cx.has_ui() {
        return Err(
            "Applying a move requires interactive confirmation; use --dry-run to preview.".into(),
        );
    }
    if !ui::confirm("Apply project state re-link?", &text)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(());
    }
    validate(&old_root, &new_root, exists(&old_root).await)?;
    apply_all(&plan, &old_root, &new_root, &roots).await;
    Ok(())
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

    #[test]
    fn parses_arguments() {
        assert_eq!(parse_args("").unwrap(), (None, false));
        assert_eq!(
            parse_args("--dry-run ~/old").unwrap(),
            (Some("~/old".into()), true)
        );
        assert_eq!(
            parse_args("\"/a b/c\"").unwrap(),
            (Some("/a b/c".into()), false)
        );
        assert_eq!(parse_args("'/x y'").unwrap(), (Some("/x y".into()), false));
        assert_eq!(
            parse_args("--force").unwrap_err(),
            "Unknown option: --force"
        );
        assert_eq!(parse_args("a b").unwrap_err(), USAGE);
    }

    #[test]
    fn validates_old_roots() {
        assert!(
            validate("/a", "/a", Some(false))
                .unwrap_err()
                .contains("same")
        );
        assert!(
            validate("/a", "/b", Some(true))
                .unwrap_err()
                .contains("copy, not a move")
        );
        assert!(validate("/a", "/b", None).is_ok());
        assert!(validate("/a", "/b", Some(false)).is_ok());
    }

    #[test]
    fn candidates_prefer_the_cwd_name() {
        let c = sort_candidates(
            vec![
                "/z/other".into(),
                "/b/abc".into(),
                "/a/abc".into(),
                "/a/abc".into(),
            ],
            "/new/abc",
        );
        assert_eq!(c, ["/a/abc", "/b/abc", "/z/other"]);
    }

    #[test]
    fn plan_groups_by_area() {
        let items = vec![
            Item::GitRepair {
                description: "repair".into(),
            },
            Item::Handler {
                extension: "unipi.memory".into(),
                lines: vec!["memory: a → b".into(), "  • x → y".into()],
            },
            Item::Manual {
                area: "pi-trust".into(),
                description: "accept".into(),
            },
        ];
        let p = plan_text("/old", "/new", &items, &["note".into()]);
        assert!(p.starts_with("Re-link /old → /new"));
        assert!(p.contains("\ngit-worktrees\n  • repair"));
        assert!(p.contains("\nunipi.memory\n  • memory: a → b\n  • x → y"));
        assert!(p.ends_with("note"));
        let e = serde_json::to_value(LogEntry::new("a", "b", "c", "d", "ok")).unwrap();
        assert!(e.get("originalHeader").is_none());
    }
}
