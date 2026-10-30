//! UNI-232: helpers for the local-only admin ops (`host_info` …).

use unipi_proto::HostTools;

/// Is `name` an executable on our PATH?
fn on_path(name: &str) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let p = dir.join(name);
        p.is_file() || (cfg!(windows) && dir.join(format!("{name}.exe")).is_file())
    })
}

/// Which tools this host can use (as `host_info` reports them).
pub fn tools() -> HostTools {
    HostTools {
        herdr: on_path("herdr"),
        tmux: on_path("tmux"),
        pi: on_path("pi"),
        kanboard: on_path("unipi-kanboard"),
    }
}
