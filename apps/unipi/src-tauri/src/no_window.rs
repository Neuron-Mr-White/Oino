//! Every child the desktop app starts runs without a console window on
//! Windows. The release app is a GUI-subsystem binary (`windows_subsystem =
//! "windows"` in main.rs), so it has no console of its own: a console child
//! (unipi-host, git, cmd, a shell, `--version` probes) started without
//! `CREATE_NO_WINDOW` gets a brand-new visible console — the black windows
//! that "look dangerous". With the flag the child gets a hidden console, and
//! its own console children (the host's git/shell/tool runs) inherit that
//! hidden console instead of opening windows. A no-op elsewhere.
//!
//! Not for PTYs: terminals go through ConPTY (portable-pty), which manages
//! its own pseudo console.

/// `CREATE_NO_WINDOW` from the Win32 process-creation flags.
#[cfg_attr(not(windows), allow(dead_code))]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `.no_window()` on std and tokio commands: no console window on Windows.
pub trait NoWindow {
    fn no_window(&mut self) -> &mut Self;
}

impl NoWindow for std::process::Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            self.creation_flags(CREATE_NO_WINDOW);
        }
        self
    }
}

impl NoWindow for tokio::process::Command {
    fn no_window(&mut self) -> &mut Self {
        #[cfg(windows)]
        self.creation_flags(CREATE_NO_WINDOW);
        self
    }
}

/// A std `Command` for `program` that never opens a console window.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> std::process::Command {
    let mut c = std::process::Command::new(program);
    c.no_window();
    c
}

/// A tokio `Command` for `program` that never opens a console window.
pub fn tokio_command(program: impl AsRef<std::ffi::OsStr>) -> tokio::process::Command {
    let mut c = tokio::process::Command::new(program);
    c.no_window();
    c
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_flag_is_create_no_window() {
        assert_eq!(super::CREATE_NO_WINDOW, 0x0800_0000);
    }
}
