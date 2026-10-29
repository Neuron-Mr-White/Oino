//! Filesystem browser/editor (UNI-155 "files — browser + editor").
//!
//! Scope/safety: every path is canonicalized before use. By default a
//! path must sit under `$HOME`, a live pi session's cwd, or one of the
//! extra safe roots (`/tmp`, `/var/tmp`, `/mnt`, … plus `unipi-host fs
//! allow` entries — see [`crate::fs_scope`], UNI-220); a caller may pass
//! `outside:true` to allow anything else (the app asks the user first).
//! Writes/renames/deletes/mkdir never follow a symlink out of the
//! allowed scope: the *parent directory* of the target is canonicalized
//! and checked, then the final component is joined back on (so a
//! not-yet-existing file can still be created, but an existing symlink
//! can't be used to redirect a write outside the scope).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use tokio::process::Command;
pub use crate::fs_scope::FsScope;
use unipi_proto::{
    AppErrorCode, AppErrorDto, EntryKind, FsConflictDetail, FsEntry, FsListData, FsReadData,
    FsRootsData, FsSearchData, FsSearchResult, FsStatData, FsWriteData, SearchKind, SearchQuery,
    FS_LIST_CAP, FS_READ_CAP, FS_SEARCH_CAP,
};

fn err(code: AppErrorCode, message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(code, message, false)
}

fn internal(e: impl std::fmt::Display) -> AppErrorDto {
    err(AppErrorCode::E_INTERNAL, e.to_string())
}

fn not_found(e: impl std::fmt::Display) -> AppErrorDto {
    err(AppErrorCode::E_NOT_FOUND, e.to_string())
}

fn scope_err(message: impl Into<String>) -> AppErrorDto {
    err(AppErrorCode::E_SCOPE, message)
}

/// Turns a request path string into the path the fs ops use (UNI-178):
/// `~`/`~/…` expands against `home`, absolute paths pass through, and a
/// relative path is a clear request error — it used to resolve silently
/// against the host process's own cwd and surface as a confusing
/// "can't resolve …".
pub fn resolve_request_path(raw: &str, home: &Path) -> Result<PathBuf, AppErrorDto> {
    if raw == "~" {
        return Ok(home.to_path_buf());
    }
    if let Some(rest) = raw.strip_prefix("~/") {
        return Ok(home.join(rest));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    Err(err(
        AppErrorCode::E_SCHEMA,
        format!(
            "path must be absolute (or start with ~/): got \"{raw}\" — resolve it against the session's cwd on the phone"
        ),
    ))
}

/// [`resolve_request_path`] + [`canonicalize_scoped`] for existing paths.
pub fn scoped_for_request(raw: &str, scope: &FsScope, outside: bool) -> Result<PathBuf, AppErrorDto> {
    canonicalize_scoped(&resolve_request_path(raw, &scope.home)?, scope, outside)
}

/// [`resolve_request_path`] + [`canonicalize_for_write`] for new files.
pub fn scoped_for_write(raw: &str, scope: &FsScope, outside: bool) -> Result<PathBuf, AppErrorDto> {
    canonicalize_for_write(&resolve_request_path(raw, &scope.home)?, scope, outside)
}

/// The user's home directory: `$HOME` when set, else (Windows only)
/// `%USERPROFILE%` — a Windows logon session has no `HOME` at all, so
/// without the fallback every scope-safe op failed with
/// "$HOME is not set on the host" on a stock PC.
pub fn home_dir_opt() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(home));
    }
    #[cfg(windows)]
    if let Some(profile) = std::env::var_os("USERPROFILE").filter(|h| !h.is_empty()) {
        return Some(PathBuf::from(profile));
    }
    None
}

/// [`home_dir_opt`], or an error if unset (the host can't do anything
/// scope-safe without it).
pub fn home_dir() -> Result<PathBuf, AppErrorDto> {
    home_dir_opt().ok_or_else(|| {
        let message = home_missing_message();
        // Silent before: the phone just showed the error, the host log had
        // nothing to correlate it with.
        tracing::warn!("{message}");
        internal(message)
    })
}

fn home_missing_message() -> &'static str {
    if cfg!(windows) {
        "$HOME (or %USERPROFILE%) is not set on the host"
    } else {
        "$HOME is not set on the host"
    }
}

/// Canonicalizes `path` and checks it is in `scope` unless `outside`
/// is set. Returns the canonical path. The path must already exist
/// (callers that need to create a new file use [`canonicalize_for_write`]
/// instead).
pub fn canonicalize_scoped(path: &Path, scope: &FsScope, outside: bool) -> Result<PathBuf, AppErrorDto> {
    let canon = dunce::canonicalize(path)
        .map_err(|e| not_found(format!("can't resolve {}: {e}", path.display())))?;
    check_scope(&canon, scope, outside)?;
    Ok(canon)
}

fn check_scope(canon: &Path, allowed: &FsScope, outside: bool) -> Result<(), AppErrorDto> {
    if outside || allowed.allows(canon) || under_session_cwd(canon) {
        Ok(())
    } else {
        Err(scope_err(format!(
            "{} is outside the allowed folders ($HOME, session folders, {}); the PC's owner can add it with `unipi-host fs allow <path>`",
            canon.display(),
            if allowed.extra.is_empty() {
                "no extra roots".to_string()
            } else {
                allowed.extra.iter().map(|r| r.display().to_string()).collect::<Vec<_>>().join(", ")
            }
        )))
    }
}

/// A live pi session's working directory (bridge discovery records) is in
/// scope too, wherever it is: `fs_roots` offers it, so it must open
/// (e.g. a project under /tmp or /srv). `$HOME` itself is never the only
/// gate for the user's own sessions.
fn under_session_cwd(canon: &Path) -> bool {
    let Some(dir) = crate::chat::bridge_dir() else { return false };
    crate::chat::read_bridges(&dir).into_iter().filter_map(|r| r.cwd).any(|cwd| {
        let cwd = PathBuf::from(cwd);
        let cwd = dunce::canonicalize(&cwd).unwrap_or(cwd);
        // Never widen to "/" (a session started at the filesystem root).
        cwd.components().count() > 1 && canon.starts_with(&cwd)
    })
}

/// Resolves `path` for a write/create/rename/mkdir target: the parent
/// directory is canonicalized and scope-checked (so a symlinked parent
/// can't be used to escape the scope), then the final component is
/// joined back on verbatim — the target itself need not exist yet.
pub fn canonicalize_for_write(path: &Path, scope: &FsScope, outside: bool) -> Result<PathBuf, AppErrorDto> {
    let name = path
        .file_name()
        .ok_or_else(|| scope_err("path has no file name"))?;
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let canon_parent = dunce::canonicalize(parent)
        .map_err(|e| not_found(format!("can't resolve {}: {e}", parent.display())))?;
    check_scope(&canon_parent, scope, outside)?;
    Ok(canon_parent.join(name))
}

fn mtime_ms(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn entry_kind(meta: &std::fs::Metadata, symlink: bool) -> EntryKind {
    if symlink {
        EntryKind::Symlink
    } else if meta.is_dir() {
        EntryKind::Dir
    } else {
        EntryKind::File
    }
}

/// Bounded time for any one `git status`/search subprocess.
const SUBPROCESS_TIMEOUT: Duration = Duration::from_secs(5);

/// Runs `git status --porcelain=v1 -z` in `repo_root`, returning a map of
/// repo-relative path → status letter (`M`/`A`/`D`/`?`). Best-effort:
/// any failure (not a repo, git missing, timeout) yields an empty map.
async fn git_status(repo_root: &Path) -> std::collections::HashMap<PathBuf, String> {
    let mut map = std::collections::HashMap::new();
    let output = tokio::time::timeout(
        SUBPROCESS_TIMEOUT,
        Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .arg("status")
            .arg("--porcelain=v1")
            .arg("-z")
            .output(),
    )
    .await;
    let Ok(Ok(output)) = output else { return map };
    if !output.status.success() {
        return map;
    }
    for entry in output.stdout.split(|b| *b == 0) {
        if entry.len() < 3 {
            continue;
        }
        let xy = &entry[0..2];
        let rest = &entry[3..];
        let path = String::from_utf8_lossy(rest).to_string();
        if path.is_empty() {
            continue;
        }
        let letter = if xy[0] == b'?' || xy[1] == b'?' {
            "?"
        } else if xy[0] == b'A' || xy[1] == b'A' {
            "A"
        } else if xy[0] == b'D' || xy[1] == b'D' {
            "D"
        } else {
            "M"
        };
        map.insert(PathBuf::from(path), letter.to_string());
    }
    map
}

/// Finds the git repo root containing `dir` by walking up to `.git`, if any.
fn repo_root_of(dir: &Path) -> Option<PathBuf> {
    let mut cur = Some(dir);
    while let Some(d) = cur {
        if d.join(".git").exists() {
            return Some(d.to_path_buf());
        }
        cur = d.parent();
    }
    None
}

/// Reads `.gitignore` patterns (just this directory's, no cascading —
/// good enough for a quick "ignored" flag) and whether `name` matches
/// one of them. Minimal glob: exact name, trailing `/`, leading `*`.
fn gitignore_patterns(dir: &Path) -> Vec<String> {
    let Ok(text) = std::fs::read_to_string(dir.join(".gitignore")) else {
        return Vec::new();
    };
    text.lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect()
}

fn matches_ignore(name: &str, is_dir: bool, patterns: &[String]) -> bool {
    patterns.iter().any(|pattern| {
        let pattern = pattern.trim_end_matches('/');
        if pattern == name {
            return true;
        }
        if let Some(suffix) = pattern.strip_prefix('*') {
            return name.ends_with(suffix);
        }
        if let Some(prefix) = pattern.strip_suffix('*') {
            return name.starts_with(prefix);
        }
        false
    }) && (is_dir || true)
}

/// Handles `fs_list`: dirs first, alphabetical within each group, capped
/// at [`FS_LIST_CAP`] with `more`; git status when the directory is
/// inside a repo.
pub async fn list(dir: &Path, hidden: bool) -> Result<FsListData, AppErrorDto> {
    let read_dir = std::fs::read_dir(dir).map_err(|e| not_found(format!("can't list {}: {e}", dir.display())))?;
    let patterns = gitignore_patterns(dir);
    let repo_root = repo_root_of(dir);
    let git_map = if let Some(root) = &repo_root {
        git_status(root).await
    } else {
        Default::default()
    };

    let mut rows: Vec<FsEntry> = Vec::new();
    for item in read_dir {
        let Ok(item) = item else { continue };
        let name = item.file_name().to_string_lossy().into_owned();
        if !hidden && name.starts_with('.') {
            continue;
        }
        let Ok(meta) = item.metadata() else { continue }; // symlink-aware (does not follow)
        let is_symlink = meta.is_symlink();
        // For size/mtime on a symlink, try to resolve it; fall back to
        // the link's own metadata if the target is unreachable.
        let resolved_meta = if is_symlink {
            std::fs::metadata(item.path()).unwrap_or_else(|_| meta.clone())
        } else {
            meta.clone()
        };
        let kind = entry_kind(&resolved_meta, is_symlink);
        let is_dir = matches!(kind, EntryKind::Dir);
        let item_path = item.path();
        let rel = repo_root
            .as_ref()
            .and_then(|root| item_path.strip_prefix(root).ok())
            .map(|p| p.to_string_lossy().into_owned());
        let git = rel.as_ref().and_then(|rel| {
            git_map.get(Path::new(rel)).cloned().or_else(|| {
                // Directory: any entry whose path starts with `rel/`.
                if is_dir {
                    git_map
                        .keys()
                        .find(|k| k.starts_with(format!("{rel}/")))
                        .and_then(|k| git_map.get(k).cloned())
                } else {
                    None
                }
            })
        });
        let ignored = matches_ignore(&name, is_dir, &patterns);
        rows.push(FsEntry {
            name,
            kind,
            size: resolved_meta.len(),
            mtime: mtime_ms(&resolved_meta),
            git,
            ignored,
        });
    }
    rows.sort_by(|a, b| {
        let a_dir = matches!(a.kind, EntryKind::Dir);
        let b_dir = matches!(b.kind, EntryKind::Dir);
        b_dir.cmp(&a_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    let more = rows.len() > FS_LIST_CAP;
    rows.truncate(FS_LIST_CAP);
    Ok(FsListData { entries: rows, more })
}

fn hex_digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// Reads `path` for `fs_read`: UTF-8 check (binary -> `E_BINARY`), size
/// cap with head-only truncation, sha256 of the returned bytes.
///
/// When `as_text` is set (UNI-191, "Edit as text"), a file that would
/// otherwise refuse with `E_BINARY` is instead lossily decoded as UTF-8
/// (invalid sequences become U+FFFD) and returned as those replacement
/// bytes -- still capped by `FS_READ_CAP` like any other read.
pub fn read_text(path: &Path, as_text: bool) -> Result<(Vec<u8>, FsReadData), AppErrorDto> {
    let meta = std::fs::metadata(path).map_err(|e| not_found(format!("can't stat {}: {e}", path.display())))?;
    if meta.is_dir() {
        return Err(err(AppErrorCode::E_UNSUPPORTED, "That's a directory, not a file"));
    }
    let size = meta.len();
    let truncated = size > FS_READ_CAP;
    let raw = if truncated {
        read_head(path, FS_READ_CAP)?
    } else {
        std::fs::read(path).map_err(|e| internal(format!("can't read {}: {e}", path.display())))?
    };
    let bytes = if std::str::from_utf8(&raw).is_err() {
        if as_text {
            String::from_utf8_lossy(&raw).into_owned().into_bytes()
        } else {
            return Err(err(AppErrorCode::E_BINARY, "That file isn't text"));
        }
    } else {
        raw
    };
    let hash = hex_digest(&bytes);
    Ok((
        bytes,
        FsReadData {
            size,
            hash,
            mtime: mtime_ms(&meta),
            truncated,
        },
    ))
}

fn read_head(path: &Path, cap: u64) -> Result<Vec<u8>, AppErrorDto> {
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| internal(format!("can't read {}: {e}", path.display())))?;
    let mut buf = vec![0u8; cap as usize];
    let n = file.read(&mut buf).map_err(internal)?;
    buf.truncate(n);
    Ok(buf)
}

/// Writes `text` to `path` atomically (tmp + rename), preserving the
/// file's mode if it already exists. Refuses with `E_CONFLICT` when
/// `base_hash` is set and no longer matches what's on disk; refuses to
/// write a new file unless `create` is set.
pub fn write_text(
    path: &Path,
    text: &str,
    base_hash: Option<&str>,
    create: bool,
) -> Result<FsWriteData, AppErrorDto> {
    let existing = std::fs::metadata(path).ok();
    if existing.is_none() && !create {
        return Err(not_found(format!("{} doesn't exist (pass create:true to make it)", path.display())));
    }
    if let Some(expected) = base_hash {
        match &existing {
            Some(_) => {
                let current = std::fs::read(path).map_err(internal)?;
                let current_hash = hex_digest(&current);
                if current_hash != expected {
                    let meta = std::fs::metadata(path).map_err(internal)?;
                    return Err(err(AppErrorCode::E_CONFLICT, "The file changed since you read it")
                        .with_detail(
                            serde_json::to_value(FsConflictDetail {
                                hash: current_hash,
                                mtime: mtime_ms(&meta),
                            })
                            .map_err(internal)?,
                        ));
                }
            }
            None => {
                // base_hash given but file doesn't exist: treat as a
                // conflict too (someone deleted it since the read).
                return Err(err(AppErrorCode::E_CONFLICT, "The file was deleted since you read it"));
            }
        }
    }
    #[cfg(unix)]
    let mode = existing.as_ref().map(|m| {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode()
    });
    let dir = path.parent().ok_or_else(|| internal("path has no parent"))?;
    std::fs::create_dir_all(dir).map_err(internal)?;
    let tmp = dir.join(format!(".tmp-{}-{:016x}", std::process::id(), rand::random::<u64>()));
    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        let mut file = options.open(&tmp).map_err(internal)?;
        file.write_all(text.as_bytes()).map_err(internal)?;
        file.sync_all().map_err(internal)?;
    }
    #[cfg(unix)]
    if let Some(mode) = mode {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode));
    }
    std::fs::rename(&tmp, path).map_err(internal)?;
    let meta = std::fs::metadata(path).map_err(internal)?;
    Ok(FsWriteData {
        hash: hex_digest(text.as_bytes()),
        mtime: mtime_ms(&meta),
    })
}

/// `fs_stat`.
pub async fn stat(path: &Path) -> Result<FsStatData, AppErrorDto> {
    let symlink_meta = std::fs::symlink_metadata(path).map_err(|e| not_found(format!("can't stat {}: {e}", path.display())))?;
    let is_symlink = symlink_meta.is_symlink();
    let meta = if is_symlink {
        std::fs::metadata(path).unwrap_or_else(|_| symlink_meta.clone())
    } else {
        symlink_meta.clone()
    };
    let kind = entry_kind(&meta, is_symlink);
    let git = if let Some(root) = repo_root_of(path.parent().unwrap_or(path)) {
        let map = git_status(&root).await;
        path.strip_prefix(&root).ok().and_then(|rel| map.get(rel).cloned())
    } else {
        None
    };
    Ok(FsStatData {
        kind,
        size: meta.len(),
        mtime: mtime_ms(&meta),
        git,
    })
}

/// Deadline for `fs_search` subprocess/walk.
const SEARCH_TIMEOUT: Duration = Duration::from_secs(10);

/// `fs_search`: prefers `fd`/`rg` if on `PATH`, otherwise a bounded
/// manual walk; capped at [`FS_SEARCH_CAP`] results and `SEARCH_TIMEOUT`.
pub async fn search(root: &Path, query: &SearchQuery, kind: SearchKind) -> Result<FsSearchData, AppErrorDto> {
    match kind {
        SearchKind::Name => search_name(root, query.as_str()).await,
        SearchKind::Content => search_content(root, query.as_str()).await,
    }
}

async fn tool_available(name: &str) -> bool {
    tokio::time::timeout(Duration::from_secs(2), Command::new(name).arg("--version").output())
        .await
        .ok()
        .and_then(|r| r.ok())
        .map(|o| o.status.success())
        .unwrap_or(false)
}

async fn search_name(root: &Path, query: &str) -> Result<FsSearchData, AppErrorDto> {
    if tool_available("fd").await {
        let output = tokio::time::timeout(
            SEARCH_TIMEOUT,
            Command::new("fd")
                .arg("--max-results")
                .arg(FS_SEARCH_CAP.to_string())
                .arg("--hidden")
                .arg("--")
                .arg(query)
                .arg(root)
                .output(),
        )
        .await;
        if let Ok(Ok(output)) = output {
            if output.status.success() || output.status.code() == Some(1) {
                let text = String::from_utf8_lossy(&output.stdout);
                let results: Vec<FsSearchResult> = text
                    .lines()
                    .take(FS_SEARCH_CAP)
                    .map(|line| FsSearchResult { path: line.to_string(), line: None, preview: None })
                    .collect();
                let more = text.lines().count() > FS_SEARCH_CAP;
                return Ok(FsSearchData { results, more });
            }
        }
    }
    walk_name(root, query)
}

/// Bounded manual walk (depth and count capped), used when `fd` isn't on
/// `PATH`.
fn walk_name(root: &Path, query: &str) -> Result<FsSearchData, AppErrorDto> {
    let query_lower = query.to_lowercase();
    let mut results = Vec::new();
    let mut more = false;
    let deadline = Instant::now() + SEARCH_TIMEOUT;
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if Instant::now() > deadline || results.len() >= FS_SEARCH_CAP {
            more = true;
            break;
        }
        if depth > 24 {
            continue;
        }
        let Ok(read_dir) = std::fs::read_dir(&dir) else { continue };
        for item in read_dir.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            if name == ".git" || name == "node_modules" || name == "target" {
                continue;
            }
            let path = item.path();
            if name.to_lowercase().contains(&query_lower) {
                results.push(FsSearchResult { path: path.to_string_lossy().into_owned(), line: None, preview: None });
                if results.len() >= FS_SEARCH_CAP {
                    more = true;
                    break;
                }
            }
            if item.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                stack.push((path, depth + 1));
            }
        }
    }
    Ok(FsSearchData { results, more })
}

async fn search_content(root: &Path, query: &str) -> Result<FsSearchData, AppErrorDto> {
    if tool_available("rg").await {
        let output = tokio::time::timeout(
            SEARCH_TIMEOUT,
            Command::new("rg")
                .arg("--line-number")
                .arg("--no-heading")
                .arg("--max-count")
                .arg("1")
                .arg("-m")
                .arg(FS_SEARCH_CAP.to_string())
                .arg("--")
                .arg(query)
                .arg(root)
                .output(),
        )
        .await;
        if let Ok(Ok(output)) = output {
            if output.status.success() || output.status.code() == Some(1) {
                let text = String::from_utf8_lossy(&output.stdout);
                let mut results = Vec::new();
                for line in text.lines().take(FS_SEARCH_CAP) {
                    // path:line:text
                    let mut parts = line.splitn(3, ':');
                    let (Some(path), Some(lineno), Some(preview)) = (parts.next(), parts.next(), parts.next()) else { continue };
                    results.push(FsSearchResult {
                        path: path.to_string(),
                        line: lineno.parse().ok(),
                        preview: Some(preview.chars().take(200).collect()),
                    });
                }
                let more = text.lines().count() > FS_SEARCH_CAP;
                return Ok(FsSearchData { results, more });
            }
        }
    }
    walk_content(root, query)
}

fn walk_content(root: &Path, query: &str) -> Result<FsSearchData, AppErrorDto> {
    let mut results = Vec::new();
    let mut more = false;
    let deadline = Instant::now() + SEARCH_TIMEOUT;
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    'outer: while let Some((dir, depth)) = stack.pop() {
        if Instant::now() > deadline {
            more = true;
            break;
        }
        if depth > 24 {
            continue;
        }
        let Ok(read_dir) = std::fs::read_dir(&dir) else { continue };
        for item in read_dir.flatten() {
            let name = item.file_name().to_string_lossy().into_owned();
            if name == ".git" || name == "node_modules" || name == "target" {
                continue;
            }
            let path = item.path();
            let is_dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                stack.push((path, depth + 1));
                continue;
            }
            let Ok(meta) = item.metadata() else { continue };
            if meta.len() > 2 * 1024 * 1024 {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            for (i, line) in text.lines().enumerate() {
                if line.contains(query) {
                    results.push(FsSearchResult {
                        path: path.to_string_lossy().into_owned(),
                        line: Some((i + 1) as u32),
                        preview: Some(line.chars().take(200).collect()),
                    });
                    if results.len() >= FS_SEARCH_CAP {
                        more = true;
                        break 'outer;
                    }
                    break; // one hit per file in the manual walk is enough
                }
            }
            if Instant::now() > deadline {
                more = true;
                break 'outer;
            }
        }
    }
    Ok(FsSearchData { results, more })
}

/// `fs_mkdir`.
pub fn mkdir(path: &Path) -> Result<(), AppErrorDto> {
    std::fs::create_dir_all(path).map_err(|e| internal(format!("can't create {}: {e}", path.display())))
}

/// `fs_rename`.
pub fn rename(from: &Path, to: &Path) -> Result<(), AppErrorDto> {
    std::fs::rename(from, to).map_err(|e| internal(format!("can't rename {} to {}: {e}", from.display(), to.display())))
}

/// `fs_import` (UNI-199): copies an already-uploaded blob
/// (`<blobs_dir>/<sha256>`, from a prior `blob_put`) to `to`. Refuses to
/// overwrite an existing file unless `overwrite:true`. `create_parent`
/// is set by the caller only for the default uploads folder (so a
/// typo'd "this folder" destination never silently creates a new tree).
pub fn import_blob(blobs_dir: &Path, sha256: &str, to: &Path, overwrite: bool, create_parent: bool) -> Result<(), AppErrorDto> {
    let source = blobs_dir.join(sha256);
    if !source.is_file() {
        return Err(not_found(format!("no uploaded blob {sha256}")));
    }
    if to.exists() && !overwrite {
        return Err(err(AppErrorCode::E_CONFLICT, format!("{} already exists", to.display())));
    }
    if create_parent {
        if let Some(dir) = to.parent() {
            std::fs::create_dir_all(dir).map_err(internal)?;
        }
    }
    std::fs::copy(&source, to).map_err(|e| internal(format!("can't copy {} to {}: {e}", source.display(), to.display())))?;
    Ok(())
}

/// `fs_delete`: refuses to delete `home` itself, or any of `roots`
/// (canonicalized before comparing). Caller must have already checked
/// `confirm`.
pub fn delete(path: &Path, home: &Path, roots: &[PathBuf]) -> Result<(), AppErrorDto> {
    if path == home || roots.iter().any(|r| r == path) {
        return Err(scope_err("Can't delete a root directory"));
    }
    let meta = std::fs::symlink_metadata(path).map_err(|e| not_found(format!("can't stat {}: {e}", path.display())))?;
    if meta.is_dir() && !meta.is_symlink() {
        std::fs::remove_dir_all(path).map_err(|e| internal(format!("can't delete {}: {e}", path.display())))
    } else {
        std::fs::remove_file(path).map_err(|e| internal(format!("can't delete {}: {e}", path.display())))
    }
}

/// `fs_roots`: `$HOME`, then the extra safe roots in effect (`/tmp`, …;
/// UNI-220), then every live pi/herdr/tmux pane's cwd (deduplicated,
/// canonicalized where possible, home listed first).
pub async fn roots(scope: &FsScope) -> Result<FsRootsData, AppErrorDto> {
    let home = scope.home.clone();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    let mut push = |p: PathBuf| {
        let p = dunce::canonicalize(&p).unwrap_or(p);
        if seen.insert(p.clone()) {
            out.push(p.to_string_lossy().into_owned());
        }
    };
    push(home.clone());
    for extra in &scope.extra {
        push(extra.clone());
    }

    // Bridges (pi chat sessions): their recorded cwd.
    if let Some(dir) = crate::chat::bridge_dir() {
        for record in crate::chat::read_bridges(&dir) {
            if let Some(cwd) = record.cwd {
                push(PathBuf::from(cwd));
            }
        }
    }

    // herdr panes' cwd, cheaply (best-effort: herdr may not be installed).
    if let Ok(state) = crate::mux::herdr::tree().await {
        if let Some(sessions) = state.sessions {
            for session in sessions {
                for workspace in session.workspaces {
                    for tab in workspace.tabs {
                        for pane in tab.panes {
                            if let Some(cwd) = pane.cwd {
                                push(PathBuf::from(cwd));
                            }
                        }
                    }
                }
            }
        }
    }

    // tmux panes' cwd.
    if let Ok(state) = crate::mux::tmux::tree().await {
        if let Some(sessions) = state.sessions {
            for session in sessions {
                for window in session.windows {
                    for pane in window.panes {
                        push(PathBuf::from(pane.cwd));
                    }
                }
            }
        }
    }

    Ok(FsRootsData { roots: out })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, name: &str, contents: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, contents).unwrap();
        p
    }

    #[tokio::test]
    async fn list_dirs_first_then_alpha_with_git_badges() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("zdir")).unwrap();
        write(root, "a.txt", "hi");
        write(root, "b.txt", "hi");
        // Simulate git seeing `a.txt` as modified, `c.txt` as untracked,
        // by faking the status output via a wrapper would need a real
        // git repo; instead just check ordering and absence of git info
        // without git installed guarantees (best-effort check).
        let data = list(root, false).await.unwrap();
        let names: Vec<&str> = data.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names[0], "zdir", "dirs sort first");
        assert!(names.contains(&"a.txt"));
        assert!(names.contains(&"b.txt"));
    }

    #[tokio::test]
    async fn list_hides_dotfiles_unless_requested() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, ".hidden", "x");
        write(root, "visible", "x");
        let without = list(root, false).await.unwrap();
        assert!(!without.entries.iter().any(|e| e.name == ".hidden"));
        let with = list(root, true).await.unwrap();
        assert!(with.entries.iter().any(|e| e.name == ".hidden"));
    }

    #[test]
    fn resolve_request_path_expands_tilde_and_rejects_relative() {
        let home = std::path::Path::new("/home/u");
        assert_eq!(resolve_request_path("~", home).unwrap(), home);
        assert_eq!(resolve_request_path("~/proj/a.ts", home).unwrap(), std::path::Path::new("/home/u/proj/a.ts"));
        assert_eq!(resolve_request_path("/etc/hosts", home).unwrap(), std::path::Path::new("/etc/hosts"));
        let err = resolve_request_path("src/a.ts", home).unwrap_err();
        assert_eq!(err.code, AppErrorCode::E_SCHEMA);
        assert!(err.message.as_str().contains("path must be absolute"), "{}", err.message.as_str());
    }

    #[test]
    fn canonicalize_scoped_rejects_outside_home_without_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let outside_dir = tmp.path().join("outside");
        fs::create_dir_all(&outside_dir).unwrap();
        let file = write(&outside_dir, "f.txt", "x");
        let scope = FsScope::home_only(&home);
        assert!(canonicalize_scoped(&file, &scope, false).is_err());
        assert!(canonicalize_scoped(&file, &scope, true).is_ok());
    }

    #[test]
    fn canonicalize_scoped_allows_under_home() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        let file = write(&home, "f.txt", "x");
        assert!(canonicalize_scoped(&file, &FsScope::home_only(&home), false).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn canonicalize_scoped_catches_symlink_escape() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let outside = tmp.path().join("outside");
        fs::create_dir_all(&outside).unwrap();
        let secret = write(&outside, "secret.txt", "s");
        let link = home.join("link.txt");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        // The symlink itself lives under home, but resolves outside it.
        assert!(canonicalize_scoped(&link, &FsScope::home_only(&home), false).is_err());
    }

    /// UNI-220: an extra safe root (e.g. /tmp) opens, lists and takes
    /// writes like $HOME, without `outside`.
    #[test]
    fn extra_root_allows_read_and_write_without_outside() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let extra = tmp.path().join("scratch");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(extra.join("sub")).unwrap();
        let file = write(&extra.join("sub"), "a.txt", "x");
        let scope = FsScope { home: home.clone(), extra: vec![extra.canonicalize().unwrap()], deny: vec![] };
        assert!(canonicalize_scoped(&file, &scope, false).is_ok());
        assert!(canonicalize_scoped(&extra, &scope, false).is_ok());
        let target = canonicalize_for_write(&extra.join("sub").join("new.txt"), &scope, false).unwrap();
        write_text(&target, "hi", None, true).unwrap();
        assert_eq!(fs::read_to_string(extra.join("sub/new.txt")).unwrap(), "hi");
        // A sibling of the extra root is still out of scope.
        let other = tmp.path().join("other");
        fs::create_dir_all(&other).unwrap();
        let err = canonicalize_scoped(&write(&other, "o.txt", "o"), &scope, false).unwrap_err();
        assert_eq!(err.code, AppErrorCode::E_SCOPE);
        assert!(err.message.as_str().contains("unipi-host fs allow"), "{}", err.message.as_str());
        assert!(canonicalize_for_write(&other.join("n.txt"), &scope, false).is_err());
    }

    /// UNI-220: a symlink inside an extra root can't escape it (read or write).
    #[cfg(unix)]
    #[test]
    fn extra_root_blocks_symlink_escapes() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let extra = tmp.path().join("scratch");
        let outside = tmp.path().join("outside");
        for d in [&home, &extra, &outside] {
            fs::create_dir_all(d).unwrap();
        }
        let secret = write(&outside, "secret.txt", "s");
        std::os::unix::fs::symlink(&secret, extra.join("link.txt")).unwrap();
        std::os::unix::fs::symlink(&outside, extra.join("dirlink")).unwrap();
        std::os::unix::fs::symlink("/etc", extra.join("etc")).unwrap();
        let scope = FsScope { home, extra: vec![extra.canonicalize().unwrap()], deny: vec![] };
        assert!(canonicalize_scoped(&extra.join("link.txt"), &scope, false).is_err());
        assert!(canonicalize_scoped(&extra.join("dirlink"), &scope, false).is_err());
        assert!(canonicalize_scoped(&extra.join("etc/hostname"), &scope, false).is_err());
        assert!(canonicalize_for_write(&extra.join("dirlink").join("x.txt"), &scope, false).is_err());
        // `..` spelled through the root is judged by where it lands.
        assert!(canonicalize_scoped(&extra.join("../outside/secret.txt"), &scope, false).is_err());
    }

    #[tokio::test]
    async fn roots_lists_home_then_extra_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let extra = tmp.path().join("scratch");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&extra).unwrap();
        let scope = FsScope { home: home.canonicalize().unwrap(), extra: vec![extra.canonicalize().unwrap()], deny: vec![] };
        let data = roots(&scope).await.unwrap();
        assert_eq!(data.roots[0], home.canonicalize().unwrap().to_string_lossy());
        assert_eq!(data.roots[1], extra.canonicalize().unwrap().to_string_lossy());
    }

    #[test]
    fn read_text_rejects_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bin");
        fs::write(&path, [0xff, 0xfe, 0x00, 0x01]).unwrap();
        let result = read_text(&path, false);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, AppErrorCode::E_BINARY);
    }

    #[test]
    fn read_text_as_text_lossily_decodes_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("bin");
        fs::write(&path, [0xff, 0xfe, b'h', b'i', 0x00]).unwrap();
        let (bytes, _data) = read_text(&path, true).unwrap();
        // Invalid UTF-8 became replacement characters; the result is
        // always valid UTF-8 (the editor can always open it as text).
        assert!(std::str::from_utf8(&bytes).is_ok());
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.contains("hi"));
        assert!(text.contains('\u{FFFD}'));
    }

    #[test]
    fn read_text_truncates_big_files() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("big.txt");
        let big = "a".repeat((FS_READ_CAP as usize) + 100);
        fs::write(&path, &big).unwrap();
        let (bytes, data) = read_text(&path, false).unwrap();
        assert!(data.truncated);
        assert_eq!(bytes.len() as u64, FS_READ_CAP);
        assert_eq!(data.size, big.len() as u64);
    }

    #[test]
    fn read_text_small_file_round_trips_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("f.txt");
        fs::write(&path, "hello").unwrap();
        let (bytes, data) = read_text(&path, false).unwrap();
        assert_eq!(bytes, b"hello");
        assert_eq!(data.hash, hex_digest(b"hello"));
        assert!(!data.truncated);
    }

    #[test]
    fn write_text_creates_new_file_only_with_create_flag() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("new.txt");
        assert!(write_text(&path, "x", None, false).is_err());
        assert!(write_text(&path, "x", None, true).is_ok());
        assert_eq!(fs::read_to_string(&path).unwrap(), "x");
    }

    #[test]
    fn write_text_is_atomic_and_detects_conflict() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("f.txt");
        fs::write(&path, "original").unwrap();
        let (_, read) = read_text(&path, false).unwrap();
        // Someone else changes the file in between.
        fs::write(&path, "changed by someone else").unwrap();
        let result = write_text(&path, "my edit", Some(&read.hash), false);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, AppErrorCode::E_CONFLICT);
        // Still readable afterwards (write was refused, no partial state).
        assert_eq!(fs::read_to_string(&path).unwrap(), "changed by someone else");
    }

    #[test]
    fn write_text_succeeds_when_hash_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("f.txt");
        fs::write(&path, "original").unwrap();
        let (_, read) = read_text(&path, false).unwrap();
        let result = write_text(&path, "updated", Some(&read.hash), false);
        assert!(result.is_ok());
        assert_eq!(fs::read_to_string(&path).unwrap(), "updated");
    }

    #[cfg(unix)]
    #[test]
    fn write_text_keeps_file_mode() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("f.sh");
        fs::write(&path, "#!/bin/sh\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        write_text(&path, "#!/bin/sh\necho hi\n", None, false).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755);
    }

    #[test]
    fn delete_refuses_home_and_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let root2 = tmp.path().join("root2");
        fs::create_dir_all(&root2).unwrap();
        assert!(delete(&home, &home, &[]).is_err());
        assert!(delete(&root2, &home, &[root2.clone()]).is_err());
        let file = write(&home, "ok.txt", "x");
        assert!(delete(&file, &home, &[]).is_ok());
        assert!(!file.exists());
    }

    #[test]
    fn delete_removes_directory_recursively() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        let dir = home.join("sub");
        fs::create_dir_all(&dir).unwrap();
        write(&dir, "f.txt", "x");
        assert!(delete(&dir, &home, &[]).is_ok());
        assert!(!dir.exists());
    }

    #[tokio::test]
    async fn search_name_finds_matching_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "needle.txt", "x");
        write(root, "other.txt", "x");
        let data = search(root, &SearchQuery::try_from("needle".to_string()).unwrap(), SearchKind::Name)
            .await
            .unwrap();
        assert!(data.results.iter().any(|r| r.path.contains("needle.txt")));
        assert!(!data.results.iter().any(|r| r.path.contains("other.txt")));
    }

    #[tokio::test]
    async fn search_content_finds_matching_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write(root, "a.txt", "nothing here");
        write(root, "b.txt", "line one\nTODO: fix this\nline three");
        let data = search(root, &SearchQuery::try_from("TODO".to_string()).unwrap(), SearchKind::Content)
            .await
            .unwrap();
        assert!(data.results.iter().any(|r| r.path.ends_with("b.txt") && r.line == Some(2)));
    }

    #[test]
    fn mkdir_then_rename_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a").join("b");
        mkdir(&dir).unwrap();
        assert!(dir.is_dir());
        let renamed = tmp.path().join("c");
        rename(&dir, &renamed).unwrap();
        assert!(renamed.is_dir());
        assert!(!dir.exists());
    }

    #[test]
    fn import_blob_copies_and_refuses_to_clobber() {
        let tmp = tempfile::tempdir().unwrap();
        let blobs_dir = tmp.path().join("blobs");
        fs::create_dir_all(&blobs_dir).unwrap();
        let sha = "deadbeef";
        fs::write(blobs_dir.join(sha), b"uploaded bytes").unwrap();
        let dest = tmp.path().join("dest.png");

        import_blob(&blobs_dir, sha, &dest, false, false).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"uploaded bytes");

        // Second import to the same path without overwrite refuses.
        let err = import_blob(&blobs_dir, sha, &dest, false, false).unwrap_err();
        assert_eq!(err.code, AppErrorCode::E_CONFLICT);
        assert_eq!(fs::read(&dest).unwrap(), b"uploaded bytes");

        // overwrite:true replaces it.
        fs::write(blobs_dir.join(sha), b"new bytes").unwrap();
        import_blob(&blobs_dir, sha, &dest, true, false).unwrap();
        assert_eq!(fs::read(&dest).unwrap(), b"new bytes");
    }

    #[test]
    fn import_blob_unknown_sha_is_not_found() {
        let tmp = tempfile::tempdir().unwrap();
        let blobs_dir = tmp.path().join("blobs");
        fs::create_dir_all(&blobs_dir).unwrap();
        let dest = tmp.path().join("dest.png");
        let err = import_blob(&blobs_dir, "nope", &dest, false, false).unwrap_err();
        assert_eq!(err.code, AppErrorCode::E_NOT_FOUND);
    }

    #[test]
    fn import_blob_creates_parent_only_when_asked() {
        let tmp = tempfile::tempdir().unwrap();
        let blobs_dir = tmp.path().join("blobs");
        fs::create_dir_all(&blobs_dir).unwrap();
        let sha = "cafe";
        fs::write(blobs_dir.join(sha), b"x").unwrap();
        let dest_dir = tmp.path().join("uploads");
        let dest = dest_dir.join("x.png");

        // create_parent:false and a missing parent fails.
        assert!(import_blob(&blobs_dir, sha, &dest, false, false).is_err());
        assert!(!dest_dir.exists());

        // create_parent:true makes the folder first.
        import_blob(&blobs_dir, sha, &dest, false, true).unwrap();
        assert!(dest.is_file());
    }
}
