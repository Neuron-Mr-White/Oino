//! Reads a PipeWire screen-cast node as raw BGRx frames through a
//! `gst-launch-1.0 pipewiresrc` child (no libpipewire link, so the static
//! musl build keeps working). Shared by the `portal` backend (the node lives
//! behind the portal's PipeWire fd, passed as fd 3) and the `mutter` backend
//! (the node is on the session's own PipeWire daemon, no fd).

use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;

use tokio::io::AsyncReadExt;
use tokio::sync::watch;
use unipi_proto::AppErrorDto;

use super::config::unavailable;
use super::encode::Frame;

/// The command line for the frame reader (pure, tested).
/// The output is forced to `width×height`: on HiDPI the PipeWire buffers can
/// differ from the size we were told, and the reader slices stdout into
/// fixed-size frames. `with_fd`: the node is reached through fd 3 (portal).
pub fn gst_args(node: u32, fps: u32, width: u32, height: u32, with_fd: bool) -> Vec<String> {
    let caps = format!("video/x-raw,format=BGRx,width={width},height={height}");
    let max_rate = format!("max-rate={fps}");
    let mut args: Vec<String> = vec!["-q".into(), "pipewiresrc".into()];
    if with_fd {
        args.push("fd=3".into());
    }
    args.extend(
        [
            &format!("path={node}"),
            "always-copy=true",
            "do-timestamp=true",
            "keepalive-time=1000",
            "!",
            "videorate",
            "drop-only=true",
            &max_rate,
            "!",
            "videoconvert",
            "!",
            "videoscale",
            "!",
            &caps,
            "!",
            "fdsink",
            "fd=1",
            "sync=false",
        ]
        .iter()
        .map(|s| s.to_string()),
    );
    args
}

/// Spawns `gst-launch-1.0` (with the PipeWire fd as fd 3 when given) and
/// pumps its raw BGRx frames into `tx`. Dropping `tx` (gst exited) ends the
/// capture.
pub fn spawn_reader(
    fd: Option<OwnedFd>,
    node: u32,
    fps: u32,
    width: u32,
    height: u32,
    tx: watch::Sender<Option<Arc<Frame>>>,
) -> Result<tokio::process::Child, AppErrorDto> {
    let gst = which("gst-launch-1.0").ok_or_else(|| unavailable("no_gstreamer", "gst-launch-1.0 (GStreamer) is not installed on the PC"))?;
    let mut command = tokio::process::Command::new(gst);
    command
        .args(gst_args(node, fps, width, height, fd.is_some()))
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    if let Some(fd) = fd.as_ref() {
        let raw = fd.as_raw_fd();
        // SAFETY: only async-signal-safe libc calls between fork and exec.
        unsafe {
            command.pre_exec(move || {
                if c_dup2(raw, 3) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn().map_err(|e| unavailable("no_gstreamer", format!("Could not start gst-launch-1.0: {e}")))?;
    drop(fd);
    let mut stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    tokio::spawn(async move {
        use tokio::io::AsyncBufReadExt;
        let mut lines = tokio::io::BufReader::new(stderr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            tracing::warn!(op = "desktop", "gst: {line}");
        }
    });
    let size = (width * height * 4) as usize;
    tokio::spawn(async move {
        let mut buf = vec![0u8; size];
        loop {
            if stdout.read_exact(&mut buf).await.is_err() {
                break; // gst exited: dropping tx ends the stream
            }
            if tx.send(Some(Arc::new(Frame::new(width, height, buf.clone())))).is_err() {
                break;
            }
        }
    });
    Ok(child)
}

extern "C" {
    #[link_name = "dup2"]
    fn c_dup2(old: i32, new: i32) -> i32;
}

pub fn which(name: &str) -> Option<std::path::PathBuf> {
    std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|dir| dir.join(name)).find(|p| p.is_file()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gst_command() {
        let args = gst_args(57, 30, 1920, 1080, true);
        assert!(args.contains(&"path=57".to_string()));
        assert!(args.contains(&"fd=3".to_string()));
        assert!(args.contains(&"video/x-raw,format=BGRx,width=1920,height=1080".to_string()));
        assert!(args.contains(&"max-rate=30".to_string()));
        let args = gst_args(57, 30, 1920, 1080, false);
        assert!(!args.iter().any(|a| a.starts_with("fd=3")), "mutter: the session's own PipeWire, no fd");
        assert!(args.contains(&"path=57".to_string()));
    }
}
