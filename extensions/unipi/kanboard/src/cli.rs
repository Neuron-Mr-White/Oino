//! The kanboard bridge (ext-abi WP-X3): runs a `unipi-kanboard` argv through
//! `process.exec` — one process per call, `--json` on the wire. The binary is
//! resolved by the host's process service (`OINO_EXT_BIN_DIR` → the package's
//! `bin/` → `PATH`); `UNIPI_KANBOARD_BIN` hands the agent an absolute-path
//! override. Writes still serialise on the board's file locks (the extension,
//! the agent's own calls and the Board share them).

use serde_json::Value;

/// Default CLI timeout.
pub const DEFAULT_TIMEOUT_MS: u64 = 10_000;

/// A failure the binary (or the spawn) reported; the message is meant to be shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CliError {
    /// Message.
    pub message: String,
    /// Exit code (1 when unknown).
    pub code: i32,
    /// `rule`, `usage`, `not_found`, `io` or `parse`.
    pub kind: String,
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl CliError {
    /// An I/O failure (spawn, permission, timeout).
    #[must_use]
    pub fn io(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: 1,
            kind: "io".into(),
        }
    }
}

/// The unavailable message (unipi `unavailableMessage`).
#[must_use]
pub fn unavailable_message(platform: &str, arch: &str) -> String {
    format!("kanboard binary unavailable for {platform}-{arch}")
}

fn decode(stderr: &str, stdout: &str) -> Option<Value> {
    [stderr, stdout].into_iter().find_map(|c| {
        let t = c.trim();
        t.starts_with('{')
            .then(|| serde_json::from_str(t).ok())
            .flatten()
    })
}

/// Interprets one finished run (pure; unipi `createCli` callback).
pub fn interpret(
    stdout: &str,
    stderr: &str,
    code: Option<i32>,
    killed: bool,
    json: bool,
) -> Result<Value, CliError> {
    let out = stdout.trim();
    let err = stderr.trim();
    let failed = killed || code != Some(0);
    if failed {
        let code = code.unwrap_or(1);
        if out.is_empty() && err.is_empty() {
            let why = if killed {
                "timed out".to_owned()
            } else {
                format!("exited with code {code}")
            };
            return Err(CliError {
                message: format!("unipi-kanboard {why}"),
                code,
                kind: "io".into(),
            });
        }
        if let Some(payload) = decode(err, out)
            && payload.get("error").is_some()
        {
            return Err(CliError {
                message: payload
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("unipi-kanboard failed")
                    .to_owned(),
                code,
                kind: payload
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("rule")
                    .to_owned(),
            });
        }
        let message = if err.is_empty() { out } else { err };
        return Err(CliError {
            message: message.to_owned(),
            code,
            kind: "io".into(),
        });
    }
    if !json {
        return Ok(Value::String(out.to_owned()));
    }
    serde_json::from_str(out).map_err(|_| CliError {
        message: format!(
            "unexpected output from unipi-kanboard: {}",
            out.chars().take(200).collect::<String>()
        ),
        code: 1,
        kind: "parse".into(),
    })
}

/// argv with `--json` appended when JSON is wanted and missing.
#[must_use]
pub fn argv(args: &[String], json: bool) -> Vec<String> {
    let mut out = args.to_vec();
    if json && !out.iter().any(|a| a == "--json") {
        out.push("--json".into());
    }
    out
}

/// Runs `unipi-kanboard args` as a child process. `env` goes on top of the
/// host environment; the actor defaults to `user` (the extension acts as the
/// user; only the agent's own calls say `agent`). `binary` is the resolved
/// command (`UNIPI_KANBOARD_BIN` override or the bare name); `timeout_ms`
/// defaults to [`DEFAULT_TIMEOUT_MS`].
pub async fn run(
    binary: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
    json: bool,
    timeout_ms: Option<u64>,
) -> Result<Value, CliError> {
    let mut vars = env.to_vec();
    if !vars.iter().any(|(k, _)| k == "UNIPI_KANBOARD_ACTOR") {
        vars.push(("UNIPI_KANBOARD_ACTOR".to_owned(), "user".to_owned()));
    }
    let full = argv(args, json);
    let refs: Vec<&str> = full.iter().map(String::as_str).collect();
    let out = oino_extension::process::exec_env(
        binary,
        &refs,
        cwd,
        &vars,
        timeout_ms.or(Some(DEFAULT_TIMEOUT_MS)),
    )
    .await
    .map_err(|e| CliError::io(e.to_string()))?;
    interpret(&out.stdout, &out.stderr, out.code, out.killed, json)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn parses_json_and_adds_the_flag() {
        assert_eq!(argv(&["list".into()], true), ["list", "--json"]);
        assert_eq!(
            argv(&["list".into(), "--json".into()], true),
            ["list", "--json"]
        );
        assert_eq!(argv(&["list".into()], false), ["list"]);
        assert_eq!(
            interpret("{\"a\":1}", "", Some(0), false, true).unwrap(),
            json!({"a": 1})
        );
        assert_eq!(
            interpret("hi\n", "", Some(0), false, false).unwrap(),
            json!("hi")
        );
    }

    #[test]
    fn surfaces_rule_messages_and_kinds() {
        let e = interpret(
            "",
            r#"{"ok":false,"error":"in_review → todo requires --comment","kind":"rule"}"#,
            Some(1),
            false,
            true,
        )
        .unwrap_err();
        assert_eq!(
            (e.message.as_str(), e.code, e.kind.as_str()),
            ("in_review → todo requires --comment", 1, "rule")
        );
        let e = interpret("", "boom", Some(2), false, true).unwrap_err();
        assert_eq!((e.message.as_str(), e.kind.as_str()), ("boom", "io"));
        let e = interpret("", "", None, true, true).unwrap_err();
        assert_eq!(e.message, "unipi-kanboard timed out");
        let e = interpret("not json", "", Some(0), false, true).unwrap_err();
        assert_eq!(e.kind, "parse");
        assert_eq!(
            unavailable_message("linux", "x86_64"),
            "kanboard binary unavailable for linux-x86_64"
        );
    }
}
