//! Hidden Xvfb display for headed browsers on Linux (browser-control.md §4.4, unipi
//! `display.rs`): a private display from `:99` up, shared by every profile, stopped with the
//! service.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

/// A running Xvfb.
#[derive(Debug)]
pub(crate) struct Display {
    number: u32,
    child: std::sync::Mutex<Option<tokio::process::Child>>,
}

impl Display {
    /// `:N`.
    pub(crate) fn var(&self) -> String {
        format!(":{}", self.number)
    }

    fn alive(&self) -> bool {
        let mut guard = self
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .as_mut()
            .is_some_and(|c| matches!(c.try_wait(), Ok(None)))
    }

    fn stop(&self) {
        let mut guard = self
            .child
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(mut child) = guard.take() {
            let _ = child.start_kill();
        }
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        self.stop();
    }
}

/// `Xvfb` on `PATH`.
pub(crate) fn xvfb() -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join("Xvfb"))
        .find(|p| p.is_file())
}

fn free(n: u32) -> bool {
    !Path::new(&format!("/tmp/.X11-unix/X{n}")).exists()
        && !Path::new(&format!("/tmp/.X{n}-lock")).exists()
}

/// Lazily started shared display.
#[derive(Debug, Default)]
pub(crate) struct Displays {
    current: Mutex<Option<Arc<Display>>>,
}

impl Displays {
    /// The running display, starting one if needed.
    pub(crate) async fn ensure(&self) -> Result<Arc<Display>, String> {
        let mut current = self.current.lock().await;
        if let Some(d) = current.as_ref().filter(|d| d.alive()) {
            return Ok(Arc::clone(d));
        }
        let program = xvfb().ok_or("Xvfb is not installed")?;
        let number = (99..199).find(|n| free(*n)).ok_or("no free X display")?;
        let mut cmd = tokio::process::Command::new(program);
        cmd.arg(format!(":{number}"))
            .args(["-screen", "0", "1920x1080x24", "-nolisten", "tcp"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let child = cmd.spawn().map_err(|e| format!("Xvfb: {e}"))?;
        let display = Arc::new(Display {
            number,
            child: std::sync::Mutex::new(Some(child)),
        });
        let socket = PathBuf::from(format!("/tmp/.X11-unix/X{number}"));
        for _ in 0..50 {
            if socket.exists() {
                *current = Some(Arc::clone(&display));
                return Ok(display);
            }
            if !display.alive() {
                return Err("Xvfb exited at start".into());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("Xvfb did not start".into())
    }

    /// Stops the display.
    pub(crate) async fn stop(&self) {
        if let Some(d) = self.current.lock().await.take() {
            d.stop();
        }
    }
}
