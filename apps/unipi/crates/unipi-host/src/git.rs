//! Basic git on the PC (UNI-217): the `git` op shells out to the `git` CLI
//! for the repo containing a folder in the Files scope.
//!
//! Safety:
//! - `path` goes through the same scope as `fs_*` ([`crate::fs::scoped_for_request`]);
//!   the repo's work-tree root must be in scope too (no `git init /` backdoor).
//! - Never interactive: stdin is `/dev/null`, `GIT_TERMINAL_PROMPT=0`,
//!   `GCM_INTERACTIVE=never`, ssh runs with `BatchMode=yes`, `GIT_EDITOR=true`.
//!   A remote that needs a password fails with `E_GIT` + a plain hint.
//! - Every command has a timeout (15 s local, 60 s commit, 120 s network);
//!   the whole process group is killed when it runs out.
//! - File paths are repo-relative (`GitPath`: no `/…`, no `..`) and passed
//!   as `:(literal)` pathspecs after `--`; refs can't start with `-`.
//! - `discard` needs `confirm:true`; untracked files it deletes must resolve
//!   inside the work tree.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncReadExt;
use tokio::process::Command;
use unipi_proto::{
    AppErrorCode, AppErrorDto, GitArea, GitBranch, GitBranchFile, GitBranchFilesData,
    GitBranchesData, GitCmd, GitCommitData, GitCommitInfo, GitDiffData, GitFileChange, GitLogData,
    GitPath, GitRepoData, GitShortstatData, GitShowData, GitShowFile, GitStatusData, GitSyncData,
    GitTurnFile, GitTurnFilesData, GIT_DIFF_CAP, GIT_SHORTSTAT_UNTRACKED_MAX,
};

use crate::fs::FsScope;

const LOCAL_TIMEOUT: Duration = Duration::from_secs(15);
const COMMIT_TIMEOUT: Duration = Duration::from_secs(60);
const NETWORK_TIMEOUT: Duration = Duration::from_secs(120);
/// Most changed files `status` lists.
pub const STATUS_CAP: usize = 2000;
/// Untracked files larger than this get no line count.
const COUNT_LINES_MAX: u64 = 1024 * 1024;
const LOG_DEFAULT: u32 = 50;

fn err(code: AppErrorCode, message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(code, message, false)
}

fn json<T: serde::Serialize>(value: T) -> Result<serde_json::Value, AppErrorDto> {
    serde_json::to_value(value).map_err(|e| err(AppErrorCode::E_INTERNAL, e.to_string()))
}

/// What one git run produced.
#[derive(Debug)]
pub struct Out {
    pub code: i32,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Out {
    pub(crate) fn ok(&self) -> bool {
        self.code == 0
    }
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

/// A repo (or a folder that may be one) plus how to run git there.
pub struct Git {
    pub dir: PathBuf,
}

impl Git {
    fn command(&self, args: &[&str]) -> Command {
        let mut cmd =
            Command::new(std::env::var_os("UNIPI_GIT_BIN").unwrap_or_else(|| "git".into()));
        cmd.arg("--no-optional-locks")
            .args([
                "-c",
                "color.ui=false",
                "-c",
                "core.quotepath=false",
                "-c",
                "core.pager=cat",
                "-c",
                "advice.detachedHead=false",
            ])
            .arg("-C")
            .arg(&self.dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .env("GIT_EDITOR", "true")
            .env("GIT_SEQUENCE_EDITOR", "true")
            .env("GIT_MERGE_AUTOEDIT", "no")
            .env("GIT_PAGER", "cat")
            .env("LC_ALL", "C")
            .env("SSH_ASKPASS_REQUIRE", "never")
            .env_remove("GIT_ASKPASS")
            .env_remove("SSH_ASKPASS")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        if std::env::var_os("GIT_SSH_COMMAND").is_none() && std::env::var_os("GIT_SSH").is_none() {
            // Keep a configured core.sshCommand working: it is only the
            // default when no env override is set.
            cmd.env(
                "GIT_SSH_COMMAND",
                "ssh -o BatchMode=yes -o ConnectTimeout=15",
            );
        }
        cmd
    }

    /// Runs git, returning its output whatever the exit code; only a
    /// missing binary or the timeout are errors here.
    pub async fn run(&self, args: &[&str], limit: Duration) -> Result<Out, AppErrorDto> {
        let mut child = self.command(args).spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                err(
                    AppErrorCode::E_UNSUPPORTED,
                    "git isn't installed on this PC",
                )
                .with_hint("Install git on the PC, then try again.")
            } else {
                err(AppErrorCode::E_INTERNAL, format!("couldn't start git: {e}"))
            }
        })?;
        let pid = child.id();
        let mut stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");
        let work = async {
            let mut out = Vec::new();
            let mut errb = Vec::new();
            let (a, b) = tokio::join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut errb));
            a.and(b)?;
            let status = child.wait().await?;
            Ok::<_, std::io::Error>((status, out, errb))
        };
        match tokio::time::timeout(limit, work).await {
            Ok(Ok((status, stdout, stderr))) => Ok(Out {
                code: status.code().unwrap_or(-1),
                stdout,
                stderr: String::from_utf8_lossy(&stderr).into_owned(),
            }),
            Ok(Err(e)) => Err(err(AppErrorCode::E_INTERNAL, format!("git failed: {e}"))),
            Err(_) => {
                kill_group(pid);
                Err(AppErrorDto::new(
                    AppErrorCode::E_TIMEOUT,
                    format!("git {} took longer than {} s and was stopped", args.first().copied().unwrap_or(""), limit.as_secs()),
                    true,
                )
                .with_hint("A hook, a slow remote or a credential prompt can hang git. Try it once in a terminal on the PC."))
            }
        }
    }

    /// [`run`](Self::run), but a non-zero exit is an `E_GIT` error.
    pub async fn ok(&self, args: &[&str], limit: Duration) -> Result<Out, AppErrorDto> {
        let out = self.run(args, limit).await?;
        if out.ok() {
            Ok(out)
        } else {
            Err(git_error(&out))
        }
    }
}

fn kill_group(pid: Option<u32>) {
    #[cfg(unix)]
    if let Some(pid) = pid {
        let _ = std::process::Command::new("kill")
            .args(["-KILL", "--", &format!("-{pid}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// git's failure as one plain `E_GIT` error: its last meaningful lines plus
/// a hint for the usual suspects (auth, identity, diverged, dirty tree).
pub fn git_error(out: &Out) -> AppErrorDto {
    let raw = if out.stderr.trim().is_empty() {
        String::from_utf8_lossy(&out.stdout).into_owned()
    } else {
        out.stderr.clone()
    };
    let lines: Vec<&str> = raw
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty() && !l.starts_with("hint:"))
        .collect();
    let mut message = lines
        .iter()
        .rev()
        .take(6)
        .rev()
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    if message.chars().count() > 800 {
        message = message.chars().take(800).collect::<String>() + "…";
    }
    if message.is_empty() {
        message = format!("git exited with code {}", out.code);
    }
    let mut error = err(AppErrorCode::E_GIT, message);
    if let Some(hint) = hint_for(&raw) {
        error = error.with_hint(hint);
    }
    error
}

fn hint_for(text: &str) -> Option<&'static str> {
    let t = text.to_lowercase();
    let has = |s: &str| t.contains(s);
    if has("could not read username")
        || has("could not read password")
        || has("authentication failed")
        || has("terminal prompts disabled")
        || has("permission denied (publickey")
        || has("host key verification failed")
        || has("invalid username or password")
        || has("http basic: access denied")
    {
        return Some("Auth needed: this PC has no credentials for the remote that work without a prompt. Set up an SSH key or a git credential helper on the PC, then try again.");
    }
    if has("please tell me who you are")
        || has("unable to auto-detect email")
        || has("empty ident name")
    {
        return Some("Set your name and email on the PC first: git config --global user.name \"…\" and git config --global user.email \"…\".");
    }
    if has("not possible to fast-forward") || has("diverging branches") || has("have diverged") {
        return Some("Your branch and the remote both have new commits. Pull with rebase, or merge on the PC.");
    }
    if has("[rejected]") || has("non-fast-forward") || has("fetch first") {
        return Some("The remote has commits you don't have yet. Pull first, then push.");
    }
    if has("would be overwritten")
        || has("please commit your changes or stash them")
        || has("commit or stash them")
    {
        return Some("Commit or stash your changes first.");
    }
    if has("could not resolve host")
        || has("network is unreachable")
        || has("connection timed out")
        || has("connection refused")
    {
        return Some("The PC can't reach the remote right now.");
    }
    if has("nothing to commit") || has("no changes added to commit") {
        return Some("Stage some changes first.");
    }
    if has("no configured push destination")
        || has("does not appear to be a git repository")
        || has("no remote repository specified")
    {
        return Some(
            "This repo has no remote to talk to. Add one on the PC: git remote add origin <url>.",
        );
    }
    if has("there is no tracking information") {
        return Some("This branch doesn't track a remote branch yet. Push it once to set that up.");
    }
    if has("gpg failed") || has("signing failed") {
        return Some("Commit signing needs a passphrase prompt on the PC. Unlock the key there (or turn off commit.gpgsign for this repo).");
    }
    if has("unmerged") || has("conflict") {
        return Some("Resolve the conflicts on the PC (or stage the fixed files), then try again.");
    }
    None
}

/// Resolves the request path (scope-checked) to a folder git can run in.
pub fn resolve_dir(raw: &str, scope: &FsScope, outside: bool) -> Result<PathBuf, AppErrorDto> {
    let canon = crate::fs::scoped_for_request(raw, scope, outside)?;
    Ok(if canon.is_dir() {
        canon
    } else {
        canon.parent().map(Path::to_path_buf).unwrap_or(canon)
    })
}

/// Work tree root + git dir, or `None` outside a repo.
async fn locate(dir: &Path) -> Result<Option<(PathBuf, PathBuf)>, AppErrorDto> {
    let git = Git {
        dir: dir.to_path_buf(),
    };
    let out = git
        .run(
            &["rev-parse", "--show-toplevel", "--absolute-git-dir"],
            LOCAL_TIMEOUT,
        )
        .await?;
    if !out.ok() {
        return Ok(None); // not a repo (or a bare one / inside .git): nothing to show
    }
    let text = out.text();
    let mut lines = text.lines();
    let (Some(root), Some(gitdir)) = (lines.next(), lines.next()) else {
        return Ok(None);
    };
    if root.is_empty() {
        return Ok(None);
    }
    Ok(Some((PathBuf::from(root), PathBuf::from(gitdir))))
}

/// Everything about the repo except its changes.
async fn repo_info(git: &Git, gitdir: &Path) -> Result<GitRepoData, AppErrorDto> {
    let (branch, head, remotes) = tokio::join!(
        git.run(&["symbolic-ref", "-q", "--short", "HEAD"], LOCAL_TIMEOUT),
        git.run(
            &["rev-parse", "-q", "--verify", "--short", "HEAD"],
            LOCAL_TIMEOUT
        ),
        git.run(&["remote"], LOCAL_TIMEOUT),
    );
    let (branch, head, remotes) = (branch?, head?, remotes?);
    let branch_name = branch
        .ok()
        .then(|| branch.text().trim().to_string())
        .filter(|b| !b.is_empty());
    let head = head
        .ok()
        .then(|| head.text().trim().to_string())
        .filter(|h| !h.is_empty());
    let mut data = GitRepoData {
        repo: true,
        root: Some(git.dir.to_string_lossy().into_owned()),
        detached: branch_name.is_none(),
        branch: branch_name.clone(),
        head,
        remotes: remotes
            .text()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect(),
        ..Default::default()
    };
    if let Some(branch) = &branch_name {
        let refname = format!("refs/heads/{branch}");
        let out = git
            .run(
                &[
                    "for-each-ref",
                    "--format=%(upstream:short)%1f%(upstream:track,nobracket)",
                    &refname,
                ],
                LOCAL_TIMEOUT,
            )
            .await?;
        if let Some(line) = out.text().lines().next() {
            let mut parts = line.split('\u{1f}');
            let upstream = parts.next().unwrap_or("").trim();
            if !upstream.is_empty() {
                data.upstream = Some(upstream.to_string());
                let (ahead, behind) = parse_track(parts.next().unwrap_or(""));
                data.ahead = ahead;
                data.behind = behind;
            }
        }
    }
    data.stashes = std::fs::read_to_string(gitdir.join("logs/refs/stash"))
        .map(|s| s.lines().count() as u32)
        .unwrap_or(0);
    data.operation = if gitdir.join("rebase-merge").exists() || gitdir.join("rebase-apply").exists()
    {
        Some("rebase".into())
    } else if gitdir.join("MERGE_HEAD").exists() {
        Some("merge".into())
    } else if gitdir.join("CHERRY_PICK_HEAD").exists() {
        Some("cherry-pick".into())
    } else if gitdir.join("REVERT_HEAD").exists() {
        Some("revert".into())
    } else {
        None
    };
    Ok(data)
}

/// `ahead 2, behind 1` / `gone` / `` → (ahead, behind).
pub fn parse_track(track: &str) -> (u32, u32) {
    let mut ahead = 0;
    let mut behind = 0;
    for part in track.split(',') {
        let mut words = part.split_whitespace();
        match (words.next(), words.next().and_then(|n| n.parse().ok())) {
            (Some("ahead"), Some(n)) => ahead = n,
            (Some("behind"), Some(n)) => behind = n,
            _ => {}
        }
    }
    (ahead, behind)
}

/// Parses `git status --porcelain=v2 -z` entries (no branch headers).
pub fn parse_status_v2(bytes: &[u8]) -> Vec<GitFileChange> {
    let mut files = Vec::new();
    let mut records = bytes
        .split(|b| *b == 0)
        .map(|r| String::from_utf8_lossy(r).into_owned());
    let change = |path: &str, orig: Option<String>, area: GitArea, status: char| GitFileChange {
        path: path.to_string(),
        orig,
        area,
        status: status.to_string(),
        add: None,
        del: None,
        binary: false,
    };
    while let Some(rec) = records.next() {
        if rec.is_empty() {
            continue;
        }
        let kind = rec.as_bytes()[0];
        match kind {
            b'1' | b'2' => {
                let fields = if kind == b'1' { 8 } else { 9 };
                let parts: Vec<&str> = rec.splitn(fields + 1, ' ').collect();
                if parts.len() < fields + 1 {
                    continue;
                }
                let xy: Vec<char> = parts[1].chars().collect();
                let path = parts[fields];
                let orig = if kind == b'2' { records.next() } else { None };
                if xy.first().is_some_and(|c| *c != '.') {
                    files.push(change(path, orig.clone(), GitArea::Staged, xy[0]));
                }
                if xy.get(1).is_some_and(|c| *c != '.') {
                    files.push(change(path, None, GitArea::Unstaged, xy[1]));
                }
            }
            b'u' => {
                let parts: Vec<&str> = rec.splitn(11, ' ').collect();
                if let Some(path) = parts.get(10) {
                    files.push(change(path, None, GitArea::Conflict, 'U'));
                }
            }
            b'?' => {
                if let Some(path) = rec.get(2..) {
                    files.push(change(path, None, GitArea::Untracked, '?'));
                }
            }
            _ => {}
        }
    }
    files
}

/// Parses `git diff --numstat -z` into path → (add, del) (None = binary).
pub fn parse_numstat(bytes: &[u8]) -> std::collections::HashMap<String, Option<(u32, u32)>> {
    let mut map = std::collections::HashMap::new();
    let mut records = bytes
        .split(|b| *b == 0)
        .map(|r| String::from_utf8_lossy(r).into_owned());
    while let Some(rec) = records.next() {
        let rec = rec.trim_start_matches('\n').to_string();
        if rec.is_empty() {
            continue;
        }
        let mut parts = rec.splitn(3, '\t');
        let (Some(a), Some(d), Some(path)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let counts = match (a.parse::<u32>(), d.parse::<u32>()) {
            (Ok(a), Ok(d)) => Some((a, d)),
            _ => None,
        };
        let path = if path.is_empty() {
            // rename: `add\tdel\t\0old\0new\0`
            let _old = records.next();
            records.next().unwrap_or_default()
        } else {
            path.to_string()
        };
        map.insert(path, counts);
    }
    map
}

/// Line count of an untracked file (None = binary / too big / unreadable).
fn count_lines(path: &Path) -> Option<u32> {
    let meta = std::fs::symlink_metadata(path).ok()?;
    if meta.file_type().is_symlink() {
        return Some(1);
    }
    if !meta.is_file() || meta.len() > COUNT_LINES_MAX {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.iter().take(8000).any(|b| *b == 0) {
        return None;
    }
    let mut n = bytes.iter().filter(|b| **b == b'\n').count() as u32;
    if bytes.last().is_some_and(|b| *b != b'\n') {
        n += 1;
    }
    Some(n)
}

fn literal(path: &GitPath) -> String {
    format!(":(literal){}", path.as_str())
}

/// Cuts `text` at `GIT_DIFF_CAP` on a line boundary.
fn cap(text: String) -> (String, bool) {
    if text.len() <= GIT_DIFF_CAP {
        return (text, false);
    }
    let mut end = GIT_DIFF_CAP;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let cut = text[..end].rfind('\n').map(|i| i + 1).unwrap_or(end);
    (text[..cut].to_string(), true)
}

fn is_binary_diff(text: &str) -> bool {
    text.lines()
        .any(|l| l.starts_with("Binary files ") && l.ends_with(" differ"))
        || text.contains("\nGIT binary patch")
}

/// Splits a `%x1f`-separated log line into a commit.
fn parse_commit(line: &str) -> Option<GitCommitInfo> {
    let p: Vec<&str> = line.split('\u{1f}').collect();
    if p.len() < 6 {
        return None;
    }
    Some(GitCommitInfo {
        hash: p[0].trim().to_string(),
        short: p[1].to_string(),
        subject: p[2].to_string(),
        author: p[3].to_string(),
        time: p[4].parse().unwrap_or(0),
        refs: p[5]
            .split(", ")
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(String::from)
            .collect(),
    })
}

const LOG_FORMAT: &str = "--format=%H%x1f%h%x1f%s%x1f%an%x1f%at%x1f%D%x1e";

fn trimmed_output(out: &Out) -> String {
    let mut text = format!("{}\n{}", out.text().trim(), out.stderr.trim())
        .trim()
        .to_string();
    if text.chars().count() > 4000 {
        text = text.chars().take(4000).collect::<String>() + "…";
    }
    if text.is_empty() {
        text = "Done.".into();
    }
    text
}

/// Runs one `git` request: `dir` is the scope-checked folder.
pub async fn handle(
    dir: &Path,
    scope: &FsScope,
    outside: bool,
    cmd: GitCmd,
) -> Result<serde_json::Value, AppErrorDto> {
    cmd.check().map_err(|m| err(AppErrorCode::E_SCHEMA, m))?;
    let located = locate(dir).await?;
    let Some((root, gitdir)) = located else {
        if matches!(cmd, GitCmd::Repo) {
            return json(GitRepoData::default());
        }
        if matches!(cmd, GitCmd::Shortstat) {
            return json(GitShortstatData::default());
        }
        if matches!(cmd, GitCmd::TurnFiles { .. }) {
            return json(GitTurnFilesData::default());
        }
        return Err(err(
            AppErrorCode::E_GIT,
            format!("{} is not inside a git repository", dir.display()),
        ));
    };
    // The work tree itself must be in scope (a repo at / or /etc is not).
    let root = crate::fs::canonicalize_scoped(&root, scope, outside)?;
    let git = Git { dir: root.clone() };
    match cmd {
        GitCmd::Repo => json(repo_info(&git, &gitdir).await?),
        GitCmd::Status => json(status(&git, &gitdir).await?),
        GitCmd::Shortstat => json(shortstat(&git).await?),
        GitCmd::TurnFiles {
            files,
            staged,
            untracked,
        } => json(turn_files(&git, &gitdir, &files, staged, untracked).await?),
        GitCmd::BranchFiles { base } => {
            let base = base.as_ref().map(|b| b.as_str().to_string());
            json(branch_files(&git, &gitdir, base.as_deref()).await?)
        }
        GitCmd::Diff {
            file,
            staged,
            untracked,
        } => json(diff(&git, &file, staged, untracked).await?),
        GitCmd::Stage { paths, all } => {
            let mut args = vec!["add", "-A", "--"];
            let specs: Vec<String> = paths.iter().map(literal).collect();
            if all {
                args.push(":/");
            } else {
                args.extend(specs.iter().map(String::as_str));
            }
            git.ok(&args, LOCAL_TIMEOUT).await?;
            Ok(serde_json::Value::Null)
        }
        GitCmd::Unstage { paths, all } => {
            let has_head = git
                .run(&["rev-parse", "-q", "--verify", "HEAD"], LOCAL_TIMEOUT)
                .await?
                .ok();
            let specs: Vec<String> = paths.iter().map(literal).collect();
            let mut args: Vec<&str> = if has_head {
                vec!["reset", "-q", "--"]
            } else {
                vec!["rm", "--cached", "-r", "-q", "--"]
            };
            if all {
                args.push(":/");
            } else {
                args.extend(specs.iter().map(String::as_str));
            }
            let out = git.run(&args, LOCAL_TIMEOUT).await?;
            // `git reset -- paths` exits 1 when it only refreshes the index.
            if !out.ok() && !(has_head && out.code == 1 && out.stderr.trim().is_empty()) {
                return Err(git_error(&out));
            }
            Ok(serde_json::Value::Null)
        }
        GitCmd::Discard { paths, confirm } => {
            if !confirm {
                return Err(err(AppErrorCode::E_SCHEMA, "discard needs confirm:true"));
            }
            discard(&git, &root, &paths).await?;
            Ok(serde_json::Value::Null)
        }
        GitCmd::Commit { message, amend } => {
            let mut args = vec!["commit", "-q", "--cleanup=strip", "-m", message.as_str()];
            if amend {
                args.push("--amend");
            }
            git.ok(&args, COMMIT_TIMEOUT).await?;
            let out = git
                .ok(&["log", "-1", "--format=%h%x1f%s"], LOCAL_TIMEOUT)
                .await?;
            let text = out.text();
            let (hash, subject) = text.trim_end().split_once('\u{1f}').unwrap_or(("", ""));
            json(GitCommitData {
                hash: hash.to_string(),
                subject: subject.to_string(),
            })
        }
        GitCmd::Log { skip, limit } => {
            let limit = limit.unwrap_or(LOG_DEFAULT);
            if !git
                .run(&["rev-parse", "-q", "--verify", "HEAD"], LOCAL_TIMEOUT)
                .await?
                .ok()
            {
                return json(GitLogData {
                    commits: vec![],
                    more: false,
                });
            }
            let skip_arg = format!("--skip={skip}");
            let n_arg = format!("--max-count={}", limit + 1);
            let out = git
                .ok(&["log", LOG_FORMAT, &skip_arg, &n_arg], LOCAL_TIMEOUT)
                .await?;
            let text = out.text();
            let mut commits: Vec<GitCommitInfo> = text
                .split('\u{1e}')
                .filter_map(|l| parse_commit(l.trim_start_matches('\n')))
                .collect();
            let more = commits.len() > limit as usize;
            commits.truncate(limit as usize);
            json(GitLogData { commits, more })
        }
        GitCmd::Show { rev } => json(show(&git, rev.as_str()).await?),
        GitCmd::Branches => json(branches(&git).await?),
        GitCmd::Switch { branch, create } => {
            let name = branch.as_str();
            if create {
                git.ok(&["switch", "-c", name], LOCAL_TIMEOUT).await?;
            } else {
                let remote_ref = format!("refs/remotes/{name}");
                let local_ref = format!("refs/heads/{name}");
                let is_local = git
                    .run(&["show-ref", "-q", "--verify", &local_ref], LOCAL_TIMEOUT)
                    .await?
                    .ok();
                let is_remote = !is_local
                    && git
                        .run(&["show-ref", "-q", "--verify", &remote_ref], LOCAL_TIMEOUT)
                        .await?
                        .ok();
                if is_remote {
                    // origin/feat → a local `feat` tracking it (or that branch if it exists).
                    let short = name.split_once('/').map(|(_, b)| b).unwrap_or(name);
                    let short_ref = format!("refs/heads/{short}");
                    if git
                        .run(&["show-ref", "-q", "--verify", &short_ref], LOCAL_TIMEOUT)
                        .await?
                        .ok()
                    {
                        git.ok(&["switch", short], LOCAL_TIMEOUT).await?;
                    } else {
                        git.ok(&["switch", "--track", name], LOCAL_TIMEOUT).await?;
                    }
                } else {
                    git.ok(&["switch", name], LOCAL_TIMEOUT).await?;
                }
            }
            Ok(serde_json::Value::Null)
        }
        GitCmd::Fetch => {
            let out = git.ok(&["fetch", "--prune"], NETWORK_TIMEOUT).await?;
            json(GitSyncData {
                output: trimmed_output(&out),
            })
        }
        GitCmd::Pull { rebase } => {
            let args: &[&str] = if rebase {
                &["pull", "--rebase", "--autostash"]
            } else {
                &["pull", "--no-rebase", "--ff-only"]
            };
            let out = git.ok(args, NETWORK_TIMEOUT).await?;
            json(GitSyncData {
                output: trimmed_output(&out),
            })
        }
        GitCmd::Push => {
            let info = repo_info(&git, &gitdir).await?;
            let out = if info.upstream.is_none() && info.branch.is_some() {
                // First push of a branch: set its upstream on the only / `origin` remote.
                let remote = if info.remotes.iter().any(|r| r == "origin") {
                    Some("origin".to_string())
                } else if info.remotes.len() == 1 {
                    info.remotes.first().cloned()
                } else {
                    None
                };
                match remote {
                    Some(remote) => {
                        git.ok(
                            &["push", "--set-upstream", &remote, "HEAD"],
                            NETWORK_TIMEOUT,
                        )
                        .await?
                    }
                    None => git.ok(&["push"], NETWORK_TIMEOUT).await?,
                }
            } else {
                git.ok(&["push"], NETWORK_TIMEOUT).await?
            };
            json(GitSyncData {
                output: trimmed_output(&out),
            })
        }
        GitCmd::StashPush { message } => {
            let mut args = vec!["stash", "push", "--include-untracked"];
            if let Some(m) = &message {
                args.extend(["-m", m.as_str()]);
            }
            let out = git.ok(&args, LOCAL_TIMEOUT).await?;
            json(GitSyncData {
                output: trimmed_output(&out),
            })
        }
        GitCmd::StashPop => {
            let out = git.ok(&["stash", "pop"], LOCAL_TIMEOUT).await?;
            json(GitSyncData {
                output: trimmed_output(&out),
            })
        }
    }
}

async fn status(git: &Git, gitdir: &Path) -> Result<GitStatusData, AppErrorDto> {
    let (info, st, staged, unstaged) = tokio::join!(
        repo_info(git, gitdir),
        git.run(
            &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
            LOCAL_TIMEOUT
        ),
        git.run(
            &["diff", "--cached", "-M", "--numstat", "-z", "--no-ext-diff"],
            LOCAL_TIMEOUT
        ),
        git.run(&["diff", "--numstat", "-z", "--no-ext-diff"], LOCAL_TIMEOUT),
    );
    let (info, st) = (info?, st?);
    if !st.ok() {
        return Err(git_error(&st));
    }
    let staged = staged.map(|o| parse_numstat(&o.stdout)).unwrap_or_default();
    let unstaged = unstaged
        .map(|o| parse_numstat(&o.stdout))
        .unwrap_or_default();
    let mut files = parse_status_v2(&st.stdout);
    let more = files.len() > STATUS_CAP;
    files.truncate(STATUS_CAP);
    for f in files.iter_mut() {
        let counts = match f.area {
            GitArea::Staged => staged.get(&f.path).copied(),
            GitArea::Unstaged | GitArea::Conflict => unstaged.get(&f.path).copied(),
            GitArea::Untracked => Some(count_lines(&git.dir.join(&f.path)).map(|n| (n, 0))),
        };
        match counts {
            Some(Some((a, d))) => {
                f.add = Some(a);
                f.del = Some(d);
            }
            Some(None) => f.binary = true,
            None => {}
        }
    }
    Ok(GitStatusData {
        repo: info,
        files,
        more,
    })
}

/// v2 host.md §9.4: `turn_files` — the last turn's files as `status` rows,
/// each with its diff, without new snapshot machinery. The rows are
/// restricted to `files` (order kept, duplicates dropped); a file the
/// work tree no longer changed is simply absent. `staged` / `untracked`
/// narrow the set to index changes / untracked files.
async fn turn_files(
    git: &Git,
    gitdir: &Path,
    files: &[GitPath],
    staged_only: bool,
    untracked_only: bool,
) -> Result<GitTurnFilesData, AppErrorDto> {
    let st = status(git, gitdir).await?;
    let mut out: Vec<GitTurnFile> = Vec::new();
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for file in files {
        if !seen.insert(file.as_str()) {
            continue;
        }
        // A file changed in the index *and* the work tree has two rows;
        // the staged one is what a commit would take, so it wins.
        let row = [
            GitArea::Staged,
            GitArea::Unstaged,
            GitArea::Conflict,
            GitArea::Untracked,
        ]
        .into_iter()
        .filter(|area| !staged_only || *area == GitArea::Staged)
        .filter(|area| !untracked_only || *area == GitArea::Untracked)
        .find_map(|area| {
            st.files
                .iter()
                .find(|f| f.path == file.as_str() && f.area == area)
        });
        let Some(row) = row else { continue };
        let file_diff = diff(
            git,
            file,
            row.area == GitArea::Staged,
            row.area == GitArea::Untracked,
        )
        .await?;
        out.push(GitTurnFile {
            change: row.clone(),
            diff: file_diff,
        });
    }
    Ok(GitTurnFilesData {
        repo: st.repo,
        files: out,
    })
}

/// Z-96 "Branch (vs base)" (WP-A15): the files `HEAD` changed vs the base
/// ref — the given one, else the upstream tracking branch, else the first of
/// `main` / `master` that exists. Rows carry their diffs: one `--name-status`
/// run plus one whole-diff run (git emits files in the same order in both, so
/// the sections zip onto the rows), capped like every diff.
async fn branch_files(
    git: &Git,
    gitdir: &Path,
    base: Option<&str>,
) -> Result<GitBranchFilesData, AppErrorDto> {
    let info = repo_info(git, gitdir).await?;
    let has_head = git
        .run(&["rev-parse", "-q", "--verify", "HEAD"], LOCAL_TIMEOUT)
        .await?
        .ok();
    let base_ref = match base {
        Some(given) => Some(given.to_string()),
        None => {
            if let Some(upstream) = &info.upstream {
                Some(upstream.clone())
            } else if ref_exists(git, "refs/heads/main").await {
                Some("main".to_string())
            } else if ref_exists(git, "refs/heads/master").await {
                Some("master".to_string())
            } else {
                None
            }
        }
    };
    let Some(base_ref) = base_ref.filter(|_| has_head) else {
        return Ok(GitBranchFilesData {
            repo: info,
            base: None,
            files: Vec::new(),
        });
    };
    let merge_base = format!("{base_ref}...HEAD");
    let names = git
        .ok(
            &[
                "diff",
                "--name-status",
                "-M",
                "-z",
                "--no-ext-diff",
                &merge_base,
            ],
            LOCAL_TIMEOUT,
        )
        .await?;
    let numstat = git
        .ok(
            &[
                "diff",
                "--numstat",
                "-M",
                "-z",
                "--no-ext-diff",
                &merge_base,
            ],
            LOCAL_TIMEOUT,
        )
        .await?;
    let counts = parse_numstat(&numstat.stdout);
    let mut rows: Vec<GitFileChange> = Vec::new();
    let records: Vec<String> = names
        .stdout
        .split(|b| *b == 0)
        .map(|r| String::from_utf8_lossy(r).into_owned())
        .filter(|r| !r.is_empty())
        .collect();
    let mut rec = records.into_iter();
    while let Some(status) = rec.next() {
        let Some(path) = rec.next() else { break };
        if path.is_empty() {
            continue;
        }
        // rename: `R100\0old\0new\0` — the new path is the row.
        let (orig, path) = if status.starts_with('R') || status.starts_with('C') {
            let new = rec.next().unwrap_or(path.clone());
            (Some(path), new)
        } else {
            (None, path)
        };
        let status_char = status
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .chars()
            .next()
            .unwrap_or('M');
        let (add, del, binary) = match counts.get(&path) {
            Some(Some((a, d))) => (Some(*a), Some(*d), false),
            Some(None) => (None, None, true),
            None => (None, None, false),
        };
        rows.push(GitFileChange {
            path,
            orig,
            area: GitArea::Staged,
            status: status_char.to_string(),
            add,
            del,
            binary,
        });
        if rows.len() >= STATUS_CAP {
            break;
        }
    }
    let whole = git
        .run(&["diff", "-M", "--no-ext-diff", &merge_base], LOCAL_TIMEOUT)
        .await?;
    // Sections in emission order; rows and sections pair one to one until the
    // cap cuts the text — the rest show as truncated.
    let sections: Vec<String> = whole
        .text()
        .split("\ndiff --git ")
        .enumerate()
        .map(|(i, s)| {
            if i == 0 {
                s.to_string()
            } else {
                format!("diff --git {s}")
            }
        })
        .filter(|s| s.starts_with("diff --git "))
        .collect();
    let mut files = Vec::with_capacity(rows.len());
    for (i, row) in rows.into_iter().enumerate() {
        let diff = match sections.get(i) {
            Some(text) => {
                let binary = is_binary_diff(text);
                let (text, truncated) = cap(text.clone());
                GitDiffData {
                    text,
                    truncated,
                    binary,
                }
            }
            None => GitDiffData {
                text: String::new(),
                truncated: true,
                binary: row.binary,
            },
        };
        files.push(GitBranchFile { change: row, diff });
    }
    Ok(GitBranchFilesData {
        repo: info,
        base: Some(base_ref),
        files,
    })
}

/// Whether a local branch exists (base-ref defaulting).
async fn ref_exists(git: &Git, reference: &str) -> bool {
    git.run(&["rev-parse", "-q", "--verify", reference], LOCAL_TIMEOUT)
        .await
        .map(|out| out.ok())
        .unwrap_or(false)
}

/// UNI-257: total +/- of the work tree vs HEAD (staged + unstaged) plus
/// untracked files' lines (capped). Cheap: two git runs + reading at most
/// GIT_SHORTSTAT_UNTRACKED_MAX small files.
async fn shortstat(git: &Git) -> Result<GitShortstatData, AppErrorDto> {
    let has_head = git
        .run(&["rev-parse", "-q", "--verify", "HEAD"], LOCAL_TIMEOUT)
        .await?
        .ok();
    let mut tracked = std::collections::HashMap::new();
    if has_head {
        let out = git
            .ok(
                &[
                    "diff",
                    "HEAD",
                    "--numstat",
                    "-z",
                    "--no-ext-diff",
                    "--no-renames",
                ],
                LOCAL_TIMEOUT,
            )
            .await?;
        tracked = parse_numstat(&out.stdout);
    } else {
        // Before the first commit everything staged is new.
        let out = git
            .ok(
                &[
                    "diff",
                    "--cached",
                    "--numstat",
                    "-z",
                    "--no-ext-diff",
                    "--no-renames",
                ],
                LOCAL_TIMEOUT,
            )
            .await?;
        tracked.extend(parse_numstat(&out.stdout));
        let out = git
            .ok(
                &["diff", "--numstat", "-z", "--no-ext-diff", "--no-renames"],
                LOCAL_TIMEOUT,
            )
            .await?;
        for (path, counts) in parse_numstat(&out.stdout) {
            let entry = tracked.entry(path).or_insert(Some((0, 0)));
            if let (Some(e), Some(c)) = (entry.as_mut(), counts) {
                e.0 += c.0;
                e.1 += c.1;
            }
        }
    }
    let untracked = git
        .ok(
            &["ls-files", "--others", "--exclude-standard", "-z"],
            LOCAL_TIMEOUT,
        )
        .await?;
    Ok(sum_shortstat(&git.dir, &tracked, &untracked.stdout))
}

/// Pure part of [`shortstat`]: sums numstat counts and the first
/// GIT_SHORTSTAT_UNTRACKED_MAX untracked files' line counts.
pub fn sum_shortstat(
    root: &Path,
    tracked: &std::collections::HashMap<String, Option<(u32, u32)>>,
    untracked_z: &[u8],
) -> GitShortstatData {
    let mut data = GitShortstatData {
        repo: true,
        ..Default::default()
    };
    for counts in tracked.values() {
        data.files += 1;
        if let Some((a, d)) = counts {
            data.add = data.add.saturating_add(*a);
            data.del = data.del.saturating_add(*d);
        }
    }
    let paths: Vec<&[u8]> = untracked_z
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .collect();
    data.files = data.files.saturating_add(paths.len() as u32);
    data.capped = paths.len() > GIT_SHORTSTAT_UNTRACKED_MAX;
    for path in paths.into_iter().take(GIT_SHORTSTAT_UNTRACKED_MAX) {
        let rel = String::from_utf8_lossy(path);
        if let Some(n) = count_lines(&root.join(rel.as_ref())) {
            data.add = data.add.saturating_add(n);
        }
    }
    data
}

async fn diff(
    git: &Git,
    file: &GitPath,
    staged: bool,
    untracked: bool,
) -> Result<GitDiffData, AppErrorDto> {
    let spec = literal(file);
    let out = if untracked {
        // Exit 1 = "they differ" for --no-index.
        let out = git
            .run(
                &[
                    "diff",
                    "--no-index",
                    "--no-ext-diff",
                    "--",
                    "/dev/null",
                    file.as_str(),
                ],
                LOCAL_TIMEOUT,
            )
            .await?;
        if out.code > 1 {
            return Err(git_error(&out));
        }
        out
    } else if staged {
        git.ok(
            &["diff", "--cached", "-M", "--no-ext-diff", "--", &spec],
            LOCAL_TIMEOUT,
        )
        .await?
    } else {
        git.ok(&["diff", "--no-ext-diff", "--", &spec], LOCAL_TIMEOUT)
            .await?
    };
    let text = out.text();
    let binary = is_binary_diff(&text);
    let (text, truncated) = cap(text);
    Ok(GitDiffData {
        text,
        truncated,
        binary,
    })
}

async fn discard(git: &Git, root: &Path, paths: &[GitPath]) -> Result<(), AppErrorDto> {
    let specs: Vec<String> = paths.iter().map(literal).collect();
    let mut args = vec!["ls-files", "-z", "--others", "--exclude-standard", "--"];
    args.extend(specs.iter().map(String::as_str));
    let out = git.ok(&args, LOCAL_TIMEOUT).await?;
    let untracked: std::collections::HashSet<String> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|r| !r.is_empty())
        .map(|r| String::from_utf8_lossy(r).into_owned())
        .collect();
    let tracked: Vec<String> = paths
        .iter()
        .filter(|p| !untracked.contains(p.as_str()))
        .map(literal)
        .collect();
    if !tracked.is_empty() {
        let mut args = vec!["restore", "--worktree", "--"];
        args.extend(tracked.iter().map(String::as_str));
        git.ok(&args, LOCAL_TIMEOUT).await?;
    }
    for rel in &untracked {
        let target = root.join(rel);
        // Never follow a symlinked parent out of the work tree.
        let parent = target.parent().and_then(|p| dunce::canonicalize(p).ok());
        if !parent.is_some_and(|p| p.starts_with(root)) {
            return Err(err(
                AppErrorCode::E_SCOPE,
                format!("{rel} is outside the repository"),
            ));
        }
        let meta = std::fs::symlink_metadata(&target)
            .map_err(|e| err(AppErrorCode::E_NOT_FOUND, format!("{rel}: {e}")))?;
        let result = if meta.is_dir() {
            std::fs::remove_dir_all(&target)
        } else {
            std::fs::remove_file(&target)
        };
        result.map_err(|e| {
            err(
                AppErrorCode::E_INTERNAL,
                format!("couldn't delete {rel}: {e}"),
            )
        })?;
    }
    Ok(())
}

async fn show(git: &Git, rev: &str) -> Result<GitShowData, AppErrorDto> {
    let commitish = format!("{rev}^{{commit}}");
    let fmt = "--format=%H%x1f%h%x1f%s%x1f%an%x1f%at%x1f%D%x1f%b";
    let head = git
        .ok(
            &["show", "-s", fmt, "--end-of-options", &commitish],
            LOCAL_TIMEOUT,
        )
        .await?;
    let text = head.text();
    let commit = parse_commit(&text).ok_or_else(|| {
        err(
            AppErrorCode::E_GIT,
            format!("unexpected git show output for {rev}"),
        )
    })?;
    let body = text
        .splitn(7, '\u{1f}')
        .nth(6)
        .unwrap_or("")
        .trim()
        .to_string();
    let hash = commit.hash.clone();
    let names_args = [
        "show",
        "--format=",
        "-M",
        "--diff-merges=first-parent",
        "--name-status",
        "-z",
        hash.as_str(),
    ];
    let nums_args = [
        "show",
        "--format=",
        "-M",
        "--diff-merges=first-parent",
        "--numstat",
        "-z",
        hash.as_str(),
    ];
    let patch_args = [
        "show",
        "--format=",
        "-M",
        "--diff-merges=first-parent",
        "--no-ext-diff",
        hash.as_str(),
    ];
    let (names, nums, patch) = tokio::join!(
        git.ok(&names_args, LOCAL_TIMEOUT),
        git.ok(&nums_args, LOCAL_TIMEOUT),
        git.ok(&patch_args, LOCAL_TIMEOUT),
    );
    let (names, nums, patch) = (names?, nums?, patch?);
    let counts = parse_numstat(&nums.stdout);
    let mut files = Vec::new();
    let mut recs = names
        .stdout
        .split(|b| *b == 0)
        .map(|r| String::from_utf8_lossy(r).into_owned());
    while let Some(status) = recs.next() {
        let status = status.trim_start_matches('\n').to_string();
        if status.is_empty() {
            continue;
        }
        let letter = status.chars().next().unwrap_or('M');
        let path = if letter == 'R' || letter == 'C' {
            let _old = recs.next();
            recs.next().unwrap_or_default()
        } else {
            recs.next().unwrap_or_default()
        };
        let c = counts.get(&path).copied().flatten();
        files.push(GitShowFile {
            path,
            status: letter.to_string(),
            add: c.map(|c| c.0),
            del: c.map(|c| c.1),
        });
    }
    let (diff, truncated) = cap(patch.text());
    Ok(GitShowData {
        commit,
        body,
        files,
        diff,
        truncated,
    })
}

async fn branches(git: &Git) -> Result<GitBranchesData, AppErrorDto> {
    let out = git
        .ok(
            &[
                "for-each-ref",
                "--sort=-committerdate",
                "--format=%(refname)%1f%(refname:short)%1f%(HEAD)%1f%(upstream:short)%1f%(upstream:track,nobracket)%1f%(committerdate:unix)%1f%(contents:subject)",
                "refs/heads",
                "refs/remotes",
            ],
            LOCAL_TIMEOUT,
        )
        .await?;
    let mut list = Vec::new();
    for line in out.text().lines() {
        let p: Vec<&str> = line.split('\u{1f}').collect();
        if p.len() < 7 {
            continue;
        }
        let remote = p[0].starts_with("refs/remotes/");
        if remote && p[0].ends_with("/HEAD") {
            continue;
        }
        let (ahead, behind) = parse_track(p[4]);
        list.push(GitBranch {
            name: p[1].to_string(),
            remote,
            current: p[2] == "*",
            upstream: Some(p[3].to_string()).filter(|u| !u.is_empty()),
            ahead,
            behind,
            time: p[5].parse().unwrap_or(0),
            subject: p[6].to_string(),
        });
    }
    // Local first (current on top), then remotes; each newest first.
    list.sort_by_key(|b| (b.remote, !b.current));
    Ok(GitBranchesData { branches: list })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_parsing() {
        assert_eq!(parse_track("ahead 2, behind 1"), (2, 1));
        assert_eq!(parse_track("behind 3"), (0, 3));
        assert_eq!(parse_track("gone"), (0, 0));
        assert_eq!(parse_track(""), (0, 0));
    }

    #[test]
    fn status_v2_parsing() {
        let raw = b"1 M. N... 100644 100644 100644 aaa bbb a.txt\x001 .M N... 100644 100644 100644 aaa bbb b c.txt\x002 R. N... 100644 100644 100644 aaa bbb R100 new.txt\x00old.txt\x00u UU N... 100644 100644 100644 100644 a b c conf.rs\x00? new dir/x.md\x00";
        let files = parse_status_v2(raw);
        let summary: Vec<(String, GitArea, String, Option<String>)> = files
            .iter()
            .map(|f| (f.path.clone(), f.area, f.status.clone(), f.orig.clone()))
            .collect();
        assert_eq!(
            summary,
            vec![
                ("a.txt".into(), GitArea::Staged, "M".into(), None),
                ("b c.txt".into(), GitArea::Unstaged, "M".into(), None),
                (
                    "new.txt".into(),
                    GitArea::Staged,
                    "R".into(),
                    Some("old.txt".into())
                ),
                ("conf.rs".into(), GitArea::Conflict, "U".into(), None),
                ("new dir/x.md".into(), GitArea::Untracked, "?".into(), None),
            ]
        );
    }

    #[test]
    fn numstat_parsing() {
        let raw = b"3\t1\ta.txt\x00-\t-\timg.png\x000\t0\t\x00old.txt\x00new.txt\x00";
        let map = parse_numstat(raw);
        assert_eq!(map.get("a.txt"), Some(&Some((3, 1))));
        assert_eq!(map.get("img.png"), Some(&None));
        assert_eq!(map.get("new.txt"), Some(&Some((0, 0))));
    }

    #[test]
    fn hints() {
        assert!(hint_for(
            "fatal: could not read Username for 'https://github.com': terminal prompts disabled"
        )
        .unwrap()
        .starts_with("Auth needed"));
        assert!(hint_for("git@github.com: Permission denied (publickey).")
            .unwrap()
            .starts_with("Auth needed"));
        assert!(hint_for("*** Please tell me who you are.")
            .unwrap()
            .contains("user.email"));
        assert!(hint_for("fatal: Not possible to fast-forward, aborting.")
            .unwrap()
            .contains("rebase"));
        assert!(hint_for(" ! [rejected]        main -> main (fetch first)")
            .unwrap()
            .contains("Pull first"));
        assert!(hint_for("error: Please commit or stash them.")
            .unwrap()
            .contains("stash"));
        assert!(hint_for("all good").is_none());
    }

    #[test]
    fn diff_cap_cuts_on_a_line() {
        let text = "x".repeat(100) + "\n" + &"y".repeat(GIT_DIFF_CAP);
        let (cut, truncated) = cap(text);
        assert!(truncated);
        assert_eq!(cut.len(), 101);
        assert_eq!(cap("short\n".into()), ("short\n".into(), false));
    }

    #[test]
    fn error_message_drops_hints_and_keeps_tail() {
        let out = Out {
            code: 128,
            stdout: vec![],
            stderr: "hint: one\nhint: two\nfatal: bad thing\n".into(),
        };
        let e = git_error(&out);
        assert_eq!(e.code, AppErrorCode::E_GIT);
        assert_eq!(e.message.as_str(), "fatal: bad thing");
    }
}
