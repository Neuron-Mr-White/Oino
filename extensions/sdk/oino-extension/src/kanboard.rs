//! The in-process kanboard (round-5 #7): `exec` runs an `oino-kanboard` argv in
//! the host process and returns the command's `--json` payload — no daemon, no
//! HTTP port, no spawned binary. Writes still go through the library's file
//! locks, so the CLI binary, the agent's calls and extension calls serialise.

use crate::Result;
use crate::bindings::oino::extension::kanboard as wk;

/// One `exec` request: the subcommand argv, the `OINO_KANBOARD_*` environment
/// the CLI would have read, a cwd for project resolution (defaults to the
/// session cwd) and optional stdin text for `-` arguments.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// Subcommand argv (`["list", "--all"]`).
    pub argv: Vec<String>,
    /// `OINO_KANBOARD_*` variables on top of the host's environment.
    pub env: std::collections::BTreeMap<String, String>,
    /// Working directory for project resolution.
    pub cwd: Option<String>,
    /// Stdin text for `-` arguments.
    pub stdin: Option<String>,
}

/// Runs `request` in the host and returns the `--json` payload; failures come
/// back as `{ok: false, error, kind}` inside it ([`is_error`]).
pub fn exec(request: &Request) -> Result<serde_json::Value> {
    let mut env = serde_json::Map::new();
    for (k, v) in &request.env {
        env.insert(k.clone(), serde_json::Value::String(v.clone()));
    }
    let mut wire = serde_json::json!({ "argv": request.argv, "env": env });
    if let Some(cwd) = &request.cwd {
        wire["cwd"] = cwd.clone().into();
    }
    if let Some(stdin) = &request.stdin {
        wire["stdin"] = stdin.clone().into();
    }
    let wire = serde_json::to_string(&wire)?;
    let payload: serde_json::Value = serde_json::from_str(&wk::exec(&wire)?)?;
    Ok(payload)
}

/// `exec` for bare argv — `["list", "--all"]`.
pub fn run(argv: &[&str]) -> Result<serde_json::Value> {
    exec(&Request {
        argv: argv.iter().map(ToString::to_string).collect(),
        ..Request::default()
    })
}

/// Whether a payload reports a failure (`{ok: false, …}` or an `error` key).
#[must_use]
pub fn is_error(payload: &serde_json::Value) -> bool {
    payload.get("ok") == Some(&serde_json::Value::Bool(false)) || payload.get("error").is_some()
}
