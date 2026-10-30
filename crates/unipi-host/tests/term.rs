//! WP-H6 acceptance: host-owned PTY terminal service (host.md §9.3).
//!
//! Real PTYs via `portable-pty`, spawned from a tempdir data dir: open →
//! write → read, resize-throttle counts, detach/reattach continuity, kill and
//! `term.max` eviction. Unix only: the shells these tests drive (`/bin/sh`,
//! `stty`) are POSIX.
#![cfg(unix)]

use std::time::Duration;

use tempfile::TempDir;
use unipi_host::{
    data_dir::HostPaths,
    term::{Attached, OpenRequest, TermConfig, TermOutput, TermRegistry},
};

const TIMEOUT: Duration = Duration::from_secs(20);

struct Fixture {
    _dir: TempDir,
    paths: HostPaths,
    registry: TermRegistry,
}

fn fixture(max: usize) -> Fixture {
    let dir = TempDir::new().expect("tempdir");
    Fixture {
        paths: HostPaths::from_dir(dir.path()),
        registry: TermRegistry::with_max(max),
        _dir: dir,
    }
}

fn workspace() -> std::path::PathBuf {
    std::env::current_dir().expect("cwd")
}

fn open(fixture: &Fixture, cols: u16, rows: u16) -> (String, Attached) {
    let (data, attached) = fixture
        .registry
        .open(
            &fixture.paths,
            OpenRequest {
                cwd: workspace(),
                cols,
                rows,
                shell: Some("/bin/sh".into()),
            },
        )
        .expect("open terminal");
    (data.term_id.as_str().to_owned(), attached)
}

/// Reads PTY output until `needle` shows up, the child exits or the timeout
/// elapses. Returns what was read plus the exit code when the stream ended.
async fn read_until(
    attached: &mut Attached,
    needle: &str,
    timeout: Duration,
) -> (Vec<u8>, Option<Option<i64>>) {
    let deadline = tokio::time::Instant::now() + timeout;
    let mut out = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        if left.is_zero() {
            return (out, None);
        }
        match tokio::time::timeout(left, attached.recv()).await {
            Ok(Some(TermOutput::Bytes(bytes))) => {
                out.extend_from_slice(&bytes);
                if String::from_utf8_lossy(&out).contains(needle) {
                    return (out, None);
                }
            }
            Ok(Some(TermOutput::Exit(code))) => return (out, Some(code)),
            Ok(None) => return (out, None),
            Err(_) => return (out, None),
        }
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[tokio::test]
async fn open_write_read_then_exit() {
    let fixture = fixture(4);
    let (term_id, mut attached) = open(&fixture, 80, 24);

    let listed = fixture.registry.list();
    assert_eq!(listed.len(), 1, "term_list after open: {listed:?}");
    assert_eq!(listed[0].term_id.as_str(), term_id);
    assert!(listed[0].alive);
    assert_eq!(listed[0].cwd, workspace().to_string_lossy());
    assert_eq!(fixture.registry.attached_streams(&term_id), 1);

    attached.input(b"echo $((6*7))\n").expect("write input");
    let (out, exit) = read_until(&mut attached, "42", TIMEOUT).await;
    assert!(exit.is_none(), "stream ended early: {}", text(&out));
    assert!(
        text(&out).contains("42"),
        "no command output, got {:?}",
        text(&out)
    );

    attached.input(b"exit 7\n").expect("write exit");
    let (out, exit) = read_until(&mut attached, "never-printed", TIMEOUT).await;
    assert_eq!(
        exit,
        Some(Some(7)),
        "expected exit 7, got {exit:?} after {:?}",
        text(&out)
    );
    // The exited terminal stays listed (and attachable) until killed.
    let listed = fixture.registry.list();
    assert_eq!(listed.len(), 1);
    assert!(!listed[0].alive, "exited terminal must read alive=false");
}

#[tokio::test]
async fn resize_is_throttled_last_wins() {
    let fixture = fixture(4);
    let (term_id, mut attached) = open(&fixture, 80, 24);
    assert_eq!(
        fixture.registry.applied_resizes(&term_id),
        Some(0),
        "opening a PTY at a size is not a resize"
    );

    // The first request is inside an empty window: applied at once.
    attached.resize(100, 30);
    assert_eq!(
        fixture.registry.applied_resizes(&term_id),
        Some(1),
        "the first resize applies immediately"
    );

    // A burst of 20 inside one 100 ms window coalesces to one trailing apply,
    // and the last size wins.
    let started = std::time::Instant::now();
    for step in 0..20u16 {
        attached.resize(101 + step, 31 + step);
    }
    let burst = started.elapsed();
    let after_burst = fixture.registry.applied_resizes(&term_id).expect("count");
    assert!(
        after_burst <= 3,
        "a {burst:?} burst of 20 resizes applied {after_burst} times"
    );

    tokio::time::sleep(Duration::from_millis(300)).await;
    let applied = fixture.registry.applied_resizes(&term_id).expect("count");
    assert!(
        (2..=4).contains(&applied),
        "expected 1 immediate + 1 trailing apply, got {applied}"
    );

    // Last-wins, end to end: the PTY itself now reports 50 rows × 120 cols.
    attached.input(b"stty size\n").expect("write stty");
    let (out, exit) = read_until(&mut attached, "50 120", TIMEOUT).await;
    assert!(exit.is_none(), "stream ended early: {}", text(&out));
    assert!(
        text(&out).contains("50 120"),
        "PTY size is not the last requested 120×50, got {:?}",
        text(&out)
    );
}

#[tokio::test]
async fn detach_then_reattach_keeps_the_process_and_replays_nothing() {
    let fixture = fixture(4);
    let (term_id, mut first) = open(&fixture, 80, 24);

    first.input(b"X=$((6*7)); echo $X\n").expect("write");
    let (out, _) = read_until(&mut first, "42", TIMEOUT).await;
    assert!(text(&out).contains("42"), "got {:?}", text(&out));
    drop(first);
    assert_eq!(
        fixture.registry.attached_streams(&term_id),
        0,
        "dropping the stream detaches it"
    );
    assert!(
        fixture.registry.list()[0].alive,
        "detaching must not kill the shell"
    );

    // Reattach: same shell, new output only (`42` is not replayed).
    let mut second = fixture.registry.attach(&term_id, 90, 25).expect("reattach");
    second.input(b"echo $((X+1))\n").expect("write");
    let (out, exit) = read_until(&mut second, "43", TIMEOUT).await;
    assert!(exit.is_none(), "stream ended early: {}", text(&out));
    let out = text(&out);
    assert!(
        out.contains("43"),
        "the shell lost its state: expected `echo $((42+1))` to print 43, got {out:?}"
    );
    assert!(
        !out.contains("42"),
        "the host must not replay scrollback, got {out:?}"
    );
    assert_eq!(fixture.registry.attached_streams(&term_id), 1);

    // Attaching to an unknown id is E_NOT_FOUND, never a panic.
    let error = fixture
        .registry
        .attach("nope", 80, 24)
        .err()
        .expect("error");
    assert_eq!(error.code, unipi_proto::AppErrorCode::E_NOT_FOUND);
}

#[tokio::test]
async fn kill_ends_the_stream_and_forgets_the_id() {
    let fixture = fixture(4);
    let (term_id, mut attached) = open(&fixture, 80, 24);
    attached.input(b"echo alive\n").expect("write");
    let (out, _) = read_until(&mut attached, "alive", TIMEOUT).await;
    assert!(text(&out).contains("alive"), "got {:?}", text(&out));

    assert!(fixture.registry.kill(&term_id).expect("kill"));
    let (_, exit) = read_until(&mut attached, "never-printed", TIMEOUT).await;
    assert_eq!(exit, Some(None), "kill ends the stream with exit null");
    assert!(fixture.registry.list().is_empty(), "kill forgets the id");
    assert!(
        !fixture.registry.kill(&term_id).expect("idempotent"),
        "killing a forgotten terminal is a no-op, not an error"
    );
}

#[tokio::test]
async fn cap_evicts_the_oldest_idle_terminal() {
    let fixture = fixture(2);
    let (first_id, first) = open(&fixture, 80, 24);
    let (second_id, second) = open(&fixture, 80, 24);
    // `first` stays attached; `second` is the only idle terminal.
    drop(second);

    let (third_id, third) = open(&fixture, 80, 24);
    drop(third);
    let ids: Vec<String> = fixture
        .registry
        .list()
        .iter()
        .map(|info| info.term_id.as_str().to_owned())
        .collect();
    assert_eq!(
        ids,
        vec![first_id.clone(), third_id.clone()],
        "term.max=2 must evict the oldest *idle* terminal ({second_id})"
    );

    // With both terminals idle the oldest one goes.
    drop(first);
    let (fourth_id, _fourth) = open(&fixture, 80, 24);
    let ids: Vec<String> = fixture
        .registry
        .list()
        .iter()
        .map(|info| info.term_id.as_str().to_owned())
        .collect();
    assert_eq!(
        ids,
        vec![third_id, fourth_id],
        "the idle first terminal is evicted when the cap is reached"
    );
}

#[test]
fn term_config_defaults_and_clamps() {
    let dir = TempDir::new().expect("tempdir");
    let path = dir.path().join("config.json");
    assert_eq!(TermConfig::load(&path).max, 16, "missing file ⇒ 16");

    std::fs::write(&path, r#"{"term":{"max":3}}"#).expect("write");
    assert_eq!(TermConfig::load(&path).max, 3);

    std::fs::write(&path, r#"{"term":{"max":0}}"#).expect("write");
    assert_eq!(TermConfig::load(&path).max, 1, "clamped up to 1");

    std::fs::write(&path, r#"{"term":{"max":100000}}"#).expect("write");
    assert_eq!(
        TermConfig::load(&path).max,
        unipi_host::term::MAX_TERMS_LIMIT
    );

    std::fs::write(&path, "{not json").expect("write");
    assert_eq!(TermConfig::load(&path).max, 16, "corrupt ⇒ default");
}

// ---------------------------------------------------------------- wire level
// The same service over an in-memory bi-stream: `ok{data:TermAttachData}`,
// kind-1 output, kind-2 input, `resize` control, `exit` control.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use unipi_proto::{Control, Dimension, FsPath, Request, TermShell, MAX_FRAME, PROTO_VERSION};

async fn read_frame<R: AsyncRead + Unpin>(read: &mut R) -> (u8, Vec<u8>) {
    let kind = read.read_u8().await.expect("frame kind");
    let len = read.read_u32().await.expect("frame len") as usize;
    assert!(len <= MAX_FRAME);
    let mut payload = vec![0u8; len];
    read.read_exact(&mut payload).await.expect("frame payload");
    (kind, payload)
}

async fn write_frame<W: AsyncWrite + Unpin>(write: &mut W, kind: u8, payload: &[u8]) {
    let frame = unipi_proto::frame::encode(
        match kind {
            0 => unipi_proto::frame::FrameKind::Control,
            2 => unipi_proto::frame::FrameKind::Input,
            other => panic!("kind {other}"),
        },
        payload,
    );
    write.write_all(&frame).await.expect("write frame");
    write.flush().await.expect("flush");
}

async fn write_control<W: AsyncWrite + Unpin>(write: &mut W, control: &Control) {
    write_frame(write, 0, &serde_json::to_vec(control).expect("json")).await;
}

fn term_open_request() -> Request {
    Request::TermOpen {
        cwd: FsPath::try_from(workspace().to_string_lossy().into_owned()).expect("cwd"),
        cols: Dimension::try_from(80u32).expect("cols"),
        rows: Dimension::try_from(24u32).expect("rows"),
        shell: Some(TermShell::try_from("/bin/sh".to_string()).expect("shell")),
    }
}

#[tokio::test]
async fn wire_streams_input_output_resize_and_exit() {
    let fixture = fixture(4);
    let (mut client, server) = tokio::io::duplex(1 << 20);
    let (mut server_r, mut server_w) = tokio::io::split(server);
    let registry = fixture.registry.clone();
    let paths = fixture.paths.clone();
    let task = tokio::spawn(async move {
        unipi_host::term::serve_request(
            &registry,
            &paths,
            term_open_request(),
            &mut server_r,
            &mut server_w,
        )
        .await
    });

    // 1. The reply carries the new terminal's id and shell.
    let (kind, payload) = read_frame(&mut client).await;
    assert_eq!(kind, 0);
    let reply: Control = serde_json::from_slice(&payload).expect("reply");
    let Control::Ok { data: Some(data) } = reply else {
        panic!("expected ok, got {reply:?}");
    };
    let term_id = data["term_id"].as_str().expect("term_id").to_owned();
    assert_eq!(data["shell"], "/bin/sh");
    assert!(PROTO_VERSION.starts_with("2."));

    // 2. Kind-2 input reaches the PTY; its output comes back as kind-1 frames.
    write_frame(&mut client, 2, b"echo $((6*7))\n").await;
    let mut seen = Vec::new();
    while !text(&seen).contains("42") {
        let (kind, payload) = read_frame(&mut client).await;
        assert_eq!(kind, 1, "expected terminal output, got a control frame");
        seen.extend_from_slice(&payload);
    }

    // 3. A `resize` control is applied to the PTY (throttled, last-wins).
    write_control(
        &mut client,
        &Control::Resize {
            cols: Dimension::try_from(100u32).expect("cols"),
            rows: Dimension::try_from(30u32).expect("rows"),
        },
    )
    .await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    write_frame(&mut client, 2, b"stty size\n").await;
    let mut sized = Vec::new();
    while !text(&sized).contains("30 100") {
        let (kind, payload) = read_frame(&mut client).await;
        assert_eq!(kind, 1);
        sized.extend_from_slice(&payload);
    }

    // 4. The child's exit ends the stream with `exit{code}`.
    write_frame(&mut client, 2, b"exit 5\n").await;
    loop {
        let (kind, payload) = read_frame(&mut client).await;
        match kind {
            1 => {}
            0 => {
                let control: Control = serde_json::from_slice(&payload).expect("control");
                match control {
                    Control::Exit { code } => {
                        assert_eq!(code, Some(5), "exit code from the shell");
                        break;
                    }
                    other => panic!("unexpected control {other:?}"),
                }
            }
            other => panic!("unexpected frame kind {other}"),
        }
    }
    assert_eq!(
        fixture.registry.applied_resizes(&term_id),
        Some(1),
        "one attach-time resize, applied once"
    );
    assert!(task.await.expect("join").is_ok());
}

#[tokio::test]
async fn wire_list_and_kill_reply() {
    let fixture = fixture(4);
    let (term_id, attached) = open(&fixture, 80, 24);
    drop(attached);

    let (mut client, server) = tokio::io::duplex(1 << 16);
    let (mut server_r, mut server_w) = tokio::io::split(server);
    let registry = fixture.registry.clone();
    let paths = fixture.paths.clone();
    let task = tokio::spawn(async move {
        unipi_host::term::serve_request(
            &registry,
            &paths,
            Request::TermList,
            &mut server_r,
            &mut server_w,
        )
        .await
    });
    let (kind, payload) = read_frame(&mut client).await;
    assert_eq!(kind, 0);
    let Control::Ok { data: Some(data) } = serde_json::from_slice::<Control>(&payload).expect("ok")
    else {
        panic!("expected ok{{data:[…]}}");
    };
    assert_eq!(data[0]["term_id"], term_id.as_str());
    assert_eq!(data[0]["alive"], true);
    assert!(task.await.expect("join").is_ok());

    let (mut client, server) = tokio::io::duplex(1 << 16);
    let (mut server_r, mut server_w) = tokio::io::split(server);
    let registry = fixture.registry.clone();
    let paths = fixture.paths.clone();
    let kill = unipi_proto::TermId::try_from(term_id.clone()).expect("id");
    let task = tokio::spawn(async move {
        unipi_host::term::serve_request(
            &registry,
            &paths,
            Request::TermKill { term_id: kill },
            &mut server_r,
            &mut server_w,
        )
        .await
    });
    let (kind, payload) = read_frame(&mut client).await;
    assert_eq!(kind, 0);
    let Control::Ok { data } = serde_json::from_slice::<Control>(&payload).expect("ok") else {
        panic!("expected ok");
    };
    assert!(data.is_none(), "term_kill answers ok{{data:null}}");
    assert!(task.await.expect("join").is_ok());
    assert!(fixture.registry.list().is_empty());
}
