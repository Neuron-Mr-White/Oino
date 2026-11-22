use super::Args;
use std::sync::Arc;
use std::time::Duration;

use oino_client::hub::{HubOptions, SessionHub};
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::ToolSelection;
use oino_core::services::ResourceFlags;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::time::Instant;
use unipi_proto::worker::{encode, Control, Decoder, Frame, Kind, Lane, PROTOCOL};

/// The first WI-2 slice deliberately rejects session operations until the ordered
/// event/command pump and credit replenishment land. It cannot acknowledge lost work.
#[cfg(unix)]
pub(super) async fn run(args: Args) -> anyhow::Result<()> {
    use std::os::fd::{FromRawFd, OwnedFd};
    anyhow::ensure!(
        args.ctl_fd >= 3 && args.data_fd >= 3 && args.ctl_fd != args.data_fd,
        "invalid inherited worker descriptors"
    );
    // SAFETY: the hidden command transfers ownership of these distinct inherited
    // socketpair descriptors. Neither is used anywhere else in the worker.
    let ctl = unsafe { OwnedFd::from_raw_fd(args.ctl_fd) };
    let data = unsafe { OwnedFd::from_raw_fd(args.data_fd) };
    let ctl = std::os::unix::net::UnixStream::from(ctl);
    let data = std::os::unix::net::UnixStream::from(data);
    ctl.set_nonblocking(true)?;
    data.set_nonblocking(true)?;
    let ctl = tokio::net::UnixStream::from_std(ctl)?;
    let _data = tokio::net::UnixStream::from_std(data)?;
    // Parent monitoring starts before discovery/initialization and remains independent
    // of blocked guest exports and blocked data/control writes.
    let parent = args.parent_pid;
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        loop {
            tick.tick().await;
            // SAFETY: getppid is a read-only process query.
            if unsafe { libc::getppid() }.cast_unsigned() != parent {
                std::process::exit(0);
            }
        }
    });
    let hub = SessionHub::create(HubOptions {
        cwd: args.cwd.clone(),
        agent_dir: args.agent_dir.clone(),
        docs_root: None,
        session_dir: None,
        mode: ExtensionMode::Desktop,
        has_ui: true,
        approve: None,
        flags: ResourceFlags::default(),
        loader: Arc::new(crate::sessions::services::HostLoader::build(
            &args.agent_dir,
            &args.cwd,
            false,
            &[],
        )),
        models: None,
        tools: ToolSelection::Default,
        exclude_tools: Vec::new(),
    })?;
    let (mut read, mut write) = ctl.into_split();
    write_control(
        &mut write,
        &Control::Hello {
            proto: PROTOCOL,
            version: env!("CARGO_PKG_VERSION").into(),
            pid: std::process::id(),
            key: args.cwd.to_string_lossy().into_owned(),
            cwd: args.cwd.to_string_lossy().into_owned(),
            extensions: Vec::new(),
        },
    )
    .await?;
    write_control(&mut write, &Control::Ready).await?;
    let mut idle = IdleExit::new(Duration::from_secs(args.exit_grace_sec));
    let mut tick = tokio::time::interval(Duration::from_millis(100));
    // Retain one read future across ticks: cancelling mid-header would corrupt framing.
    let (incoming_tx, mut incoming) = tokio::sync::mpsc::channel(1);
    let reader = tokio::spawn(async move {
        loop {
            let message = read_control(&mut read).await;
            let done = !matches!(&message, Ok(Some(_)));
            if incoming_tx.send(message).await.is_err() || done {
                break;
            }
        }
    });
    let _reader = AbortReader(reader.abort_handle());
    loop {
        tokio::select! {
            _ = tick.tick() => {
                if idle.poll(hub.is_empty().await, Instant::now()) {
                    write_control(&mut write, &Control::Bye { reason: "idle".into() }).await?;
                    hub.reset_services().await;
                    return Ok(());
                }
            }
            message = incoming.recv() => {
                let Some(message) = message else { return Ok(()); };
                let Some(message) = message? else { return Ok(()); };
                match message {
                    Control::Ping { id } => write_control(&mut write, &Control::Pong { id }).await?,
                    Control::Shutdown { .. } => {
                        hub.reset_services().await;
                        write_control(&mut write, &Control::Bye { reason: "shutdown".into() }).await?;
                        return Ok(());
                    }
                    Control::Attach { stream, .. } | Control::Create { stream, .. } => {
                        write_control(&mut write, &Control::AttachFailed { stream, error: "WI-2 session pump not implemented".into() }).await?;
                    }
                    _ => anyhow::bail!("unsupported worker control message"),
                }
            }
        }
    }
}

#[cfg(not(unix))]
pub(super) async fn run(_: Args) -> anyhow::Result<()> {
    anyhow::bail!("worker transport is not implemented on this platform")
}

async fn read_control(read: &mut (impl AsyncRead + Unpin)) -> anyhow::Result<Option<Control>> {
    // Validate all nine header bytes before allocating or reading any payload.
    let mut header = [0; unipi_proto::worker::HEADER_LEN];
    if read.read(&mut header[..1]).await? == 0 {
        return Ok(None);
    }
    read.read_exact(&mut header[1..]).await?;
    let mut decoder = Decoder::new(Lane::Control);
    let (_, complete) = decoder.decode(&header)?;
    if let Some(frame) = complete {
        return Ok(Some(serde_json::from_slice(&frame.payload)?));
    }
    let len = u32::from_be_bytes(header[5..9].try_into()?) as usize;
    let mut bytes = vec![0; len];
    read.read_exact(&mut bytes).await?;
    let (_, frame) = decoder.decode(&bytes)?;
    let frame = frame.ok_or_else(|| anyhow::anyhow!("incomplete worker control frame"))?;
    anyhow::ensure!(
        frame.stream == 0 && frame.kind == Kind::Control,
        "expected worker control frame"
    );
    Ok(Some(serde_json::from_slice(&frame.payload)?))
}

async fn write_control(
    write: &mut (impl AsyncWrite + Unpin),
    control: &Control,
) -> anyhow::Result<()> {
    let frame = Frame {
        kind: Kind::Control,
        stream: 0,
        payload: serde_json::to_vec(control)?,
    };
    write.write_all(&encode(&frame, Lane::Control)?).await?;
    Ok(())
}

struct AbortReader(tokio::task::AbortHandle);
impl Drop for AbortReader {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct IdleExit {
    grace: Duration,
    since: Option<Instant>,
}

impl IdleExit {
    fn new(grace: Duration) -> Self {
        Self { grace, since: None }
    }
    fn poll(&mut self, empty: bool, now: Instant) -> bool {
        if !empty {
            self.since = None;
            return false;
        }
        now.duration_since(*self.since.get_or_insert(now)) >= self.grace
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_requires_continuous_empty_ownership() {
        let start = Instant::now();
        let mut idle = IdleExit::new(Duration::from_secs(2));
        assert!(!idle.poll(true, start));
        assert!(!idle.poll(true, start + Duration::from_millis(1999)));
        assert!(idle.poll(true, start + Duration::from_secs(2)));
        // Any live session/services (turn, task, dialog, child) resets the grace.
        assert!(!idle.poll(false, start + Duration::from_secs(3)));
        assert!(!idle.poll(true, start + Duration::from_secs(4)));
        assert!(!idle.poll(true, start + Duration::from_secs(5)));
        assert!(idle.poll(true, start + Duration::from_secs(6)));
    }

    #[tokio::test]
    async fn control_codec_roundtrips_and_rejects_oversize_header() {
        let (mut a, mut b) = tokio::io::duplex(1024);
        write_control(&mut a, &Control::Ping { id: 42 })
            .await
            .unwrap();
        assert_eq!(
            read_control(&mut b).await.unwrap(),
            Some(Control::Ping { id: 42 })
        );
        a.write_all(&[0, 0, 0, 0, 0, 255, 255, 255, 255])
            .await
            .unwrap();
        assert!(read_control(&mut b).await.is_err());
    }
}
