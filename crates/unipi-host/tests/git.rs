//! UNI-217: the `git` op against real temp repos (git init in a tempdir),
//! called straight through `unipi_host::git::handle` plus once end to end
//! over an iroh stream. Never touches a real repo or `~/.unipi`.
#![cfg(unix)]
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use unipi_host::fs::FsScope;
use unipi_proto::{AppErrorCode, GitCmd, GitMessage, GitPath, GitRefName};

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// A repo with an identity, one commit (a.txt), on `main`.
fn repo(base: &Path, name: &str) -> PathBuf {
    let dir = base.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    git(&dir, &["config", "user.name", "Test Person"]);
    git(&dir, &["config", "user.email", "t@example.com"]);
    git(&dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    git(&dir, &["add", "a.txt"]);
    git(&dir, &["commit", "-q", "-m", "init"]);
    dir.canonicalize().unwrap()
}

fn scope_for(base: &Path) -> FsScope {
    // The temp dir stands in for $HOME so only it is in scope.
    FsScope::home_only(base.canonicalize().unwrap())
}

fn p(s: &str) -> GitPath {
    GitPath::try_from(s.to_string()).unwrap()
}

async fn run(
    dir: &Path,
    scope: &FsScope,
    cmd: GitCmd,
) -> Result<serde_json::Value, unipi_proto::AppErrorDto> {
    unipi_host::git::handle(dir, scope, false, cmd).await
}

fn isolate_env() {
    // No user/system config leaks into the test repos (signing, hooks, aliases).
    std::env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
    std::env::set_var("GIT_CONFIG_NOSYSTEM", "1");
}

#[tokio::test]
async fn repo_status_diff_stage_commit_log_show() {
    isolate_env();
    let temp = tempfile::tempdir().unwrap();
    let scope = scope_for(temp.path());
    let dir = repo(temp.path(), "r");

    // Not a repo: `repo` says so, `status` is a plain E_GIT.
    let plain = temp.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let data = run(&plain, &scope, GitCmd::Repo).await.unwrap();
    assert_eq!(data["repo"], false);
    let e = run(&plain, &scope, GitCmd::Status).await.unwrap_err();
    assert_eq!(e.code, AppErrorCode::E_GIT);

    // Repo info from a subfolder.
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let data = run(&dir.join("sub"), &scope, GitCmd::Repo).await.unwrap();
    assert_eq!(data["repo"], true);
    assert_eq!(data["branch"], "main");
    assert_eq!(data["root"], dir.to_string_lossy().as_ref());
    assert!(data.get("upstream").is_none());

    // Changes: modified a.txt (+1 -1), staged new b.txt, untracked "c d.md" (2 lines).
    std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
    std::fs::write(dir.join("b.txt"), "b\n").unwrap();
    git(&dir, &["add", "b.txt"]);
    std::fs::write(dir.join("c d.md"), "x\ny").unwrap();
    let st = run(&dir, &scope, GitCmd::Status).await.unwrap();
    let files = st["files"].as_array().unwrap();
    let find = |path: &str, area: &str| {
        files
            .iter()
            .find(|f| f["path"] == path && f["area"] == area)
            .cloned()
    };
    let a = find("a.txt", "unstaged").expect("a.txt unstaged");
    assert_eq!(
        (a["status"].as_str(), a["add"].as_u64(), a["del"].as_u64()),
        (Some("M"), Some(1), Some(1))
    );
    let b = find("b.txt", "staged").expect("b.txt staged");
    assert_eq!(
        (b["status"].as_str(), b["add"].as_u64()),
        (Some("A"), Some(1))
    );
    let c = find("c d.md", "untracked").expect("untracked");
    assert_eq!(c["add"].as_u64(), Some(2));

    // Diffs: worktree, staged, untracked.
    let d = run(
        &dir,
        &scope,
        GitCmd::Diff {
            file: p("a.txt"),
            staged: false,
            untracked: false,
        },
    )
    .await
    .unwrap();
    assert!(d["text"].as_str().unwrap().contains("-two\n+TWO"), "{d}");
    let d = run(
        &dir,
        &scope,
        GitCmd::Diff {
            file: p("b.txt"),
            staged: true,
            untracked: false,
        },
    )
    .await
    .unwrap();
    assert!(d["text"].as_str().unwrap().contains("+b"), "{d}");
    let d = run(
        &dir,
        &scope,
        GitCmd::Diff {
            file: p("c d.md"),
            staged: false,
            untracked: true,
        },
    )
    .await
    .unwrap();
    assert!(d["text"].as_str().unwrap().contains("+x\n+y"), "{d}");
    // Binary diff is flagged.
    std::fs::write(dir.join("bin.dat"), [0u8, 1, 2, 3, 0, 255]).unwrap();
    let d = run(
        &dir,
        &scope,
        GitCmd::Diff {
            file: p("bin.dat"),
            staged: false,
            untracked: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(d["binary"], true, "{d}");
    std::fs::remove_file(dir.join("bin.dat")).unwrap();

    // Stage one, unstage one, stage all.
    run(
        &dir,
        &scope,
        GitCmd::Stage {
            paths: vec![p("a.txt")],
            all: false,
        },
    )
    .await
    .unwrap();
    assert!(git(&dir, &["diff", "--cached", "--name-only"]).contains("a.txt"));
    run(
        &dir,
        &scope,
        GitCmd::Unstage {
            paths: vec![p("b.txt")],
            all: false,
        },
    )
    .await
    .unwrap();
    assert!(!git(&dir, &["diff", "--cached", "--name-only"]).contains("b.txt"));
    run(
        &dir,
        &scope,
        GitCmd::Stage {
            paths: vec![],
            all: true,
        },
    )
    .await
    .unwrap();
    let staged = git(&dir, &["diff", "--cached", "--name-only"]);
    assert!(
        staged.contains("b.txt") && staged.contains("c d.md"),
        "{staged}"
    );
    run(
        &dir,
        &scope,
        GitCmd::Unstage {
            paths: vec![],
            all: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(git(&dir, &["diff", "--cached", "--name-only"]).trim(), "");
    run(
        &dir,
        &scope,
        GitCmd::Stage {
            paths: vec![],
            all: true,
        },
    )
    .await
    .unwrap();

    // Commit, then amend.
    let msg = GitMessage::try_from("second\n\nbody line".to_string()).unwrap();
    let c = run(
        &dir,
        &scope,
        GitCmd::Commit {
            message: msg,
            amend: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(c["subject"], "second");
    let msg = GitMessage::try_from("second (amended)".to_string()).unwrap();
    let c = run(
        &dir,
        &scope,
        GitCmd::Commit {
            message: msg,
            amend: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(c["subject"], "second (amended)");
    // Nothing to commit → E_GIT with a hint.
    let msg = GitMessage::try_from("empty".to_string()).unwrap();
    let e = run(
        &dir,
        &scope,
        GitCmd::Commit {
            message: msg,
            amend: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, AppErrorCode::E_GIT);
    assert!(e.hint.is_some(), "{e:?}");

    // Log pages.
    for i in 0..5 {
        std::fs::write(dir.join("a.txt"), format!("v{i}\n")).unwrap();
        git(&dir, &["commit", "-q", "-am", &format!("c{i}")]);
    }
    let log = run(
        &dir,
        &scope,
        GitCmd::Log {
            skip: 0,
            limit: Some(3),
        },
    )
    .await
    .unwrap();
    let commits = log["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 3);
    assert_eq!(log["more"], true);
    assert_eq!(commits[0]["subject"], "c4");
    assert_eq!(commits[0]["author"], "Test Person");
    assert!(commits[0]["refs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|r| r.as_str().unwrap().contains("main")));
    let log = run(
        &dir,
        &scope,
        GitCmd::Log {
            skip: 6,
            limit: Some(3),
        },
    )
    .await
    .unwrap();
    assert_eq!(log["commits"].as_array().unwrap().len(), 1);
    assert_eq!(log["more"], false);
    assert_eq!(log["commits"][0]["subject"], "init");

    // Show a commit: files + diff.
    let hash = log["commits"][0]["hash"].as_str().unwrap().to_string();
    let rev = GitRefName::try_from(hash[..7].to_string()).unwrap();
    let show = run(&dir, &scope, GitCmd::Show { rev }).await.unwrap();
    assert_eq!(show["commit"]["subject"], "init");
    assert_eq!(show["files"][0]["path"], "a.txt");
    assert_eq!(show["files"][0]["status"], "A");
    assert_eq!(show["files"][0]["add"], 3);
    assert!(show["diff"].as_str().unwrap().contains("+one"));
    let rev = GitRefName::try_from("deadbeef".to_string()).unwrap();
    assert_eq!(
        run(&dir, &scope, GitCmd::Show { rev })
            .await
            .unwrap_err()
            .code,
        AppErrorCode::E_GIT
    );
}

#[tokio::test]
async fn shortstat_counts_staged_unstaged_and_untracked() {
    isolate_env();
    let temp = tempfile::tempdir().unwrap();
    let scope = scope_for(temp.path());
    // Not a repo: repo:false, zero counts, not an error.
    let plain = temp.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let data = run(&plain, &scope, GitCmd::Shortstat).await.unwrap();
    assert_eq!(data["repo"], false);
    assert_eq!(data["add"], 0);

    let dir = repo(temp.path(), "r");
    // Clean.
    let data = run(&dir, &scope, GitCmd::Shortstat).await.unwrap();
    assert_eq!(
        (
            data["repo"].as_bool(),
            data["files"].as_u64(),
            data["add"].as_u64(),
            data["del"].as_u64()
        ),
        (Some(true), Some(0), Some(0), Some(0))
    );
    // a.txt: one line changed and staged, then another changed unstaged → vs HEAD +2 -2.
    std::fs::write(dir.join("a.txt"), "ONE\ntwo\nthree\n").unwrap();
    git(&dir, &["add", "a.txt"]);
    std::fs::write(dir.join("a.txt"), "ONE\nTWO\nthree\n").unwrap();
    // New staged b.txt (+2), untracked c.md (+3), untracked binary (no lines).
    std::fs::write(dir.join("b.txt"), "b\nb\n").unwrap();
    git(&dir, &["add", "b.txt"]);
    std::fs::write(dir.join("c.md"), "x\ny\nz").unwrap();
    std::fs::write(dir.join("bin.dat"), [0u8, 1, 2, 3]).unwrap();
    let data = run(&dir, &scope, GitCmd::Shortstat).await.unwrap();
    assert_eq!(data["files"], 4);
    assert_eq!(data["add"], 2 + 2 + 3);
    assert_eq!(data["del"], 2);
    assert_eq!(data["capped"], false);
}

#[tokio::test]
async fn shortstat_before_the_first_commit() {
    isolate_env();
    let temp = tempfile::tempdir().unwrap();
    let scope = scope_for(temp.path());
    let dir = temp.path().join("fresh");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "-q", "-b", "main"]);
    std::fs::write(dir.join("a.txt"), "1\n2\n").unwrap();
    git(&dir, &["add", "a.txt"]);
    std::fs::write(dir.join("u.txt"), "u\n").unwrap();
    let data = run(&dir.canonicalize().unwrap(), &scope, GitCmd::Shortstat)
        .await
        .unwrap();
    assert_eq!(
        (
            data["files"].as_u64(),
            data["add"].as_u64(),
            data["del"].as_u64()
        ),
        (Some(2), Some(3), Some(0))
    );
}

#[test]
fn shortstat_caps_untracked_files() {
    let temp = tempfile::tempdir().unwrap();
    let mut z = Vec::new();
    for i in 0..(unipi_proto::GIT_SHORTSTAT_UNTRACKED_MAX + 5) {
        let name = format!("f{i}.txt");
        std::fs::write(temp.path().join(&name), "a\n").unwrap();
        z.extend_from_slice(name.as_bytes());
        z.push(0);
    }
    let tracked = std::collections::HashMap::from([
        ("x".to_string(), Some((4, 1))),
        ("img.png".to_string(), None),
    ]);
    let data = unipi_host::git::sum_shortstat(temp.path(), &tracked, &z);
    assert!(data.capped);
    assert_eq!(
        data.files as usize,
        2 + unipi_proto::GIT_SHORTSTAT_UNTRACKED_MAX + 5
    );
    assert_eq!(
        data.add as usize,
        4 + unipi_proto::GIT_SHORTSTAT_UNTRACKED_MAX
    );
    assert_eq!(data.del, 1);
}

#[tokio::test]
async fn discard_branches_switch_stash() {
    isolate_env();
    let temp = tempfile::tempdir().unwrap();
    let scope = scope_for(temp.path());
    let dir = repo(temp.path(), "r");

    std::fs::write(dir.join("a.txt"), "changed\n").unwrap();
    std::fs::write(dir.join("junk.txt"), "junk\n").unwrap();
    // Without confirm: refused, nothing touched.
    let e = run(
        &dir,
        &scope,
        GitCmd::Discard {
            paths: vec![p("a.txt")],
            confirm: false,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, AppErrorCode::E_SCHEMA);
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "changed\n"
    );
    run(
        &dir,
        &scope,
        GitCmd::Discard {
            paths: vec![p("a.txt"), p("junk.txt")],
            confirm: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("a.txt")).unwrap(),
        "one\ntwo\nthree\n"
    );
    assert!(!dir.join("junk.txt").exists());

    // Branches: create + switch, list, switch back.
    let feat = GitRefName::try_from("feat/x".to_string()).unwrap();
    run(
        &dir,
        &scope,
        GitCmd::Switch {
            branch: feat.clone(),
            create: true,
        },
    )
    .await
    .unwrap();
    assert_eq!(git(&dir, &["branch", "--show-current"]).trim(), "feat/x");
    let list = run(&dir, &scope, GitCmd::Branches).await.unwrap();
    let names: Vec<&str> = list["branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert_eq!(names[0], "feat/x", "current first: {names:?}");
    assert!(names.contains(&"main"));
    assert_eq!(list["branches"][0]["current"], true);
    let main = GitRefName::try_from("main".to_string()).unwrap();
    run(
        &dir,
        &scope,
        GitCmd::Switch {
            branch: main.clone(),
            create: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(git(&dir, &["branch", "--show-current"]).trim(), "main");
    // Creating an existing branch is a plain E_GIT.
    let e = run(
        &dir,
        &scope,
        GitCmd::Switch {
            branch: feat,
            create: true,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(e.code, AppErrorCode::E_GIT);
    assert!(e.message.as_str().contains("already exists"), "{e:?}");

    // Stash push / pop (incl. untracked).
    std::fs::write(dir.join("a.txt"), "wip\n").unwrap();
    std::fs::write(dir.join("new.txt"), "n\n").unwrap();
    run(
        &dir,
        &scope,
        GitCmd::StashPush {
            message: Some(GitMessage::try_from("wip".to_string()).unwrap()),
        },
    )
    .await
    .unwrap();
    assert!(!dir.join("new.txt").exists());
    let info = run(&dir, &scope, GitCmd::Repo).await.unwrap();
    assert_eq!(info["stashes"], 1);
    run(&dir, &scope, GitCmd::StashPop).await.unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("a.txt")).unwrap(), "wip\n");
    assert!(dir.join("new.txt").exists());
}

#[tokio::test]
async fn remote_fetch_pull_push_and_plain_errors() {
    isolate_env();
    let temp = tempfile::tempdir().unwrap();
    let scope = scope_for(temp.path());
    let origin = temp.path().join("origin.git");
    git(
        temp.path(),
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            origin.to_str().unwrap(),
        ],
    );
    let a = repo(temp.path(), "a");
    git(&a, &["remote", "add", "origin", origin.to_str().unwrap()]);

    // First push sets the upstream.
    run(&a, &scope, GitCmd::Push).await.unwrap();
    let info = run(&a, &scope, GitCmd::Repo).await.unwrap();
    assert_eq!(info["upstream"], "origin/main");
    assert_eq!(
        (info["ahead"].as_u64(), info["behind"].as_u64()),
        (Some(0), Some(0))
    );

    // A second clone pushes; `a` fetches and sees "behind 1", then pulls (ff-only).
    let b = temp.path().join("b");
    git(
        temp.path(),
        &["clone", "-q", origin.to_str().unwrap(), b.to_str().unwrap()],
    );
    git(&b, &["config", "user.name", "B"]);
    git(&b, &["config", "user.email", "b@example.com"]);
    std::fs::write(b.join("b.txt"), "b\n").unwrap();
    git(&b, &["add", "b.txt"]);
    git(&b, &["commit", "-q", "-m", "from b"]);
    git(&b, &["push", "-q"]);
    run(&a, &scope, GitCmd::Fetch).await.unwrap();
    let info = run(&a, &scope, GitCmd::Repo).await.unwrap();
    assert_eq!(info["behind"], 1);
    run(&a, &scope, GitCmd::Pull { rebase: false })
        .await
        .unwrap();
    assert!(a.join("b.txt").exists());

    // Diverged: ff-only pull fails plainly with a hint; push is rejected with one too.
    std::fs::write(b.join("b.txt"), "b2\n").unwrap();
    git(&b, &["commit", "-q", "-am", "b2"]);
    git(&b, &["push", "-q"]);
    std::fs::write(a.join("a.txt"), "a2\n").unwrap();
    git(&a, &["commit", "-q", "-am", "a2"]);
    let e = run(&a, &scope, GitCmd::Push).await.unwrap_err();
    assert_eq!(e.code, AppErrorCode::E_GIT);
    assert!(
        e.hint.as_deref().unwrap_or("").contains("Pull first"),
        "{e:?}"
    );
    run(&a, &scope, GitCmd::Fetch).await.unwrap();
    let e = run(&a, &scope, GitCmd::Pull { rebase: false })
        .await
        .unwrap_err();
    assert_eq!(e.code, AppErrorCode::E_GIT);
    assert!(e.hint.is_some(), "{e:?}");
    // Rebase pull resolves it.
    run(&a, &scope, GitCmd::Pull { rebase: true })
        .await
        .unwrap();
    run(&a, &scope, GitCmd::Push).await.unwrap();

    // An https remote that wants credentials fails fast, no prompt.
    let c = repo(temp.path(), "c");
    git(
        &c,
        &["remote", "add", "origin", "https://127.0.0.1:9/nope.git"],
    );
    let started = std::time::Instant::now();
    let e = run(&c, &scope, GitCmd::Fetch).await.unwrap_err();
    assert_eq!(e.code, AppErrorCode::E_GIT);
    assert!(started.elapsed() < Duration::from_secs(60));
    // No remote at all.
    let d = repo(temp.path(), "d");
    let e = run(&d, &scope, GitCmd::Push).await.unwrap_err();
    assert_eq!(e.code, AppErrorCode::E_GIT);
    assert!(e.hint.is_some(), "{e:?}");
}

#[tokio::test]
async fn scope_is_enforced() {
    isolate_env();
    let temp = tempfile::tempdir().unwrap();
    let inside = temp.path().join("home");
    std::fs::create_dir_all(&inside).unwrap();
    let scope = scope_for(&inside);
    // A repo outside the scoped "home".
    let outside = repo(temp.path(), "elsewhere");
    let e = unipi_host::git::resolve_dir(outside.to_str().unwrap(), &scope, false).unwrap_err();
    assert_eq!(e.code, AppErrorCode::E_SCOPE);
    // A folder inside scope whose repo root is outside it (home is a subdir of a repo).
    let r = repo(temp.path(), "outer");
    let sub = r.join("inner");
    std::fs::create_dir_all(&sub).unwrap();
    let scope = scope_for(&sub);
    let dir = unipi_host::git::resolve_dir(sub.to_str().unwrap(), &scope, false).unwrap();
    let e = run(&dir, &scope, GitCmd::Status).await.unwrap_err();
    assert_eq!(e.code, AppErrorCode::E_SCOPE);
    // A file path resolves to its folder.
    let scope = scope_for(temp.path());
    let dir =
        unipi_host::git::resolve_dir(r.join("a.txt").to_str().unwrap(), &scope, false).unwrap();
    assert_eq!(dir, r);
}

/// End to end: a paired phone sends `{op:"git"}` over a real iroh stream.
#[tokio::test]
async fn git_over_the_wire() {
    use iroh::{
        endpoint::{presets, RelayMode},
        Endpoint,
    };
    use unipi_host::{
        data_dir::HostPaths,
        endpoint::ALPN,
        pairing::{now_unix, PairingStore},
        protocol::{read_frame, Handler},
    };
    use unipi_proto::Control;
    isolate_env();
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path().join("host"));
    paths.ensure_dir().unwrap();
    // /tmp is a default extra safe root: put the repo there.
    let base = tempfile::Builder::new()
        .prefix("uni217-")
        .tempdir_in("/tmp")
        .unwrap();
    let dir = repo(base.path(), "wire");
    std::fs::write(dir.join("a.txt"), "x\n").unwrap();

    let host = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .unwrap();
    let handler = Handler {
        paths: paths.clone(),
        host_id: host.id(),
        host_name: "git-test-pc".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        cron: unipi_host::cron_serve::CronHost::new(&paths),
    };
    let _router = iroh::protocol::Router::builder(host.clone())
        .accept(ALPN, handler)
        .spawn();
    let phone = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let conn = phone.connect(host.addr(), ALPN).await.unwrap();
    let request = |req: serde_json::Value| {
        let conn = conn.clone();
        async move {
            let value =
                serde_json::json!({"type":"hello","proto":unipi_proto::PROTO_VERSION,"req":req});
            let (mut send, mut recv) = conn.open_bi().await.unwrap();
            send.write_all(&unipi_proto::frame::encode(
                unipi_proto::FrameKind::Control,
                &serde_json::to_vec(&value).unwrap(),
            ))
            .await
            .unwrap();
            send.finish().unwrap();
            let (_, bytes) = tokio::time::timeout(Duration::from_secs(20), read_frame(&mut recv))
                .await
                .unwrap()
                .unwrap();
            serde_json::from_slice::<Control>(&bytes).unwrap()
        }
    };
    let state = PairingStore::new(paths.pairing())
        .start(now_unix())
        .unwrap();
    assert!(matches!(
        request(serde_json::json!({"op":"pair","code":state.code,"name":"Pixel"})).await,
        Control::Ok { .. }
    ));

    let reply = request(serde_json::json!({"op":"git","path":dir,"cmd":{"action":"status"}})).await;
    let Control::Ok { data: Some(data) } = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(data["branch"], "main");
    assert_eq!(data["files"][0]["path"], "a.txt");
    // A system folder stays out of scope.
    let reply =
        request(serde_json::json!({"op":"git","path":"/etc","cmd":{"action":"repo"}})).await;
    let Control::Err { error } = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(error.code, AppErrorCode::E_SCOPE);
    // Bad shape (stage with nothing) is E_SCHEMA.
    let reply = request(serde_json::json!({"op":"git","path":dir,"cmd":{"action":"stage"}})).await;
    let Control::Err { error } = reply else {
        panic!("{reply:?}")
    };
    assert_eq!(error.code, AppErrorCode::E_SCHEMA);
}

/// v2 host.md §9.4 — `turn_files`: the last turn's files as status rows with
/// their diffs, restricted to the given paths (and their order), with the
/// `staged`/`untracked` narrowings; unchanged files are absent.
#[tokio::test]
async fn turn_files_restricts_status_and_diffs_to_the_given_paths() {
    isolate_env();
    let temp = tempfile::tempdir().unwrap();
    let scope = scope_for(temp.path());
    let dir = repo(temp.path(), "r");
    std::fs::write(dir.join("a.txt"), "one\nTWO\nthree\n").unwrap();
    std::fs::write(dir.join("b.txt"), "new\nfile\n").unwrap();
    std::fs::write(dir.join("untouched.txt"), "x\n").unwrap();
    git(&dir, &["add", "untouched.txt"]);
    git(&dir, &["commit", "-q", "-m", "clean file"]);

    // a.txt is changed in the work tree, b.txt is untracked; ask for both,
    // out of order, plus a clean file and a duplicate.
    let data = run(
        &dir,
        &scope,
        GitCmd::TurnFiles {
            files: vec![p("b.txt"), p("a.txt"), p("untouched.txt"), p("a.txt")],
            staged: false,
            untracked: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(data["repo"], true);
    let files = data["files"].as_array().unwrap();
    assert_eq!(files.len(), 2, "clean path is absent: {data}");
    assert_eq!(files[0]["path"], "b.txt");
    assert_eq!(files[0]["area"], "untracked");
    assert_eq!(files[0]["status"], "?");
    assert_eq!(files[1]["path"], "a.txt");
    assert_eq!(files[1]["area"], "unstaged");
    assert_eq!(files[1]["status"], "M");
    assert_eq!(files[1]["add"], 1);
    assert_eq!(files[1]["del"], 1);
    assert!(
        files[1]["diff"]["text"]
            .as_str()
            .unwrap()
            .contains("-two\n+TWO"),
        "a.txt diff: {data}"
    );
    assert!(files[0]["diff"]["text"].as_str().unwrap().contains("+new"));

    // `untracked` keeps only the untracked file.
    let data = run(
        &dir,
        &scope,
        GitCmd::TurnFiles {
            files: vec![p("a.txt"), p("b.txt")],
            staged: false,
            untracked: true,
        },
    )
    .await
    .unwrap();
    let files = data["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "b.txt");

    // Stage a.txt: `staged` now shows it from the index.
    git(&dir, &["add", "a.txt"]);
    let data = run(
        &dir,
        &scope,
        GitCmd::TurnFiles {
            staged: true,
            untracked: false,
            files: vec![p("a.txt"), p("b.txt")],
        },
    )
    .await
    .unwrap();
    let files = data["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "a.txt");
    assert_eq!(files[0]["area"], "staged");
    assert!(files[0]["diff"]["text"].as_str().unwrap().contains("+TWO"));

    // Inside no repo it is an empty answer, not an error.
    let plain = temp.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    let data = run(
        &plain,
        &scope,
        GitCmd::TurnFiles {
            files: vec![p("a.txt")],
            staged: false,
            untracked: false,
        },
    )
    .await
    .unwrap();
    assert_eq!(data["repo"], false);
    assert_eq!(data["files"].as_array().unwrap().len(), 0);
}
