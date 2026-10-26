//! The CLI bridge (unipi `bin.ts`): runs `oino-kanboard` through host `process.exec`, parses
//! its JSON and surfaces its rule messages.
//!
//! Binary resolution: `$OINO_KANBOARD_BIN` (an explicit path — tests and tinkerers), else
//! `oino-kanboard` found by the host (the Oino install dir is on the extension search path).

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
                message: format!("oino-kanboard {why}"),
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
                    .unwrap_or("oino-kanboard failed")
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
            "unexpected output from oino-kanboard: {}",
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

/// Runs the binary. `env` comes on top of the host environment; the actor defaults to
/// `user` (the extension acts as the user; only the agent's own calls say `agent`).
pub async fn run(
    binary: &str,
    args: &[String],
    env: &[(String, String)],
    cwd: Option<&str>,
    json: bool,
    timeout_ms: Option<u64>,
) -> Result<Value, CliError> {
    let mut env = env.to_vec();
    if !env.iter().any(|(k, _)| k == "OINO_KANBOARD_ACTOR") {
        env.push(("OINO_KANBOARD_ACTOR".into(), "user".into()));
    }
    let options = oino_extension::wit::process::ExecOptions {
        cwd: cwd.map(ToOwned::to_owned),
        timeout_ms: Some(timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS)),
        env,
    };
    let result = oino_extension::wit::process::exec(binary.to_owned(), argv(args, json), options)
        .await
        .map_err(|e| CliError::io(oino_extension::Error::from(e).to_string()))?;
    interpret(
        &result.stdout,
        &result.stderr,
        result.code,
        result.killed,
        json,
    )
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
        assert_eq!(e.message, "oino-kanboard timed out");
        let e = interpret("not json", "", Some(0), false, true).unwrap_err();
        assert_eq!(e.kind, "parse");
        assert_eq!(
            unavailable_message("linux", "x86_64"),
            "kanboard binary unavailable for linux-x86_64"
        );
    }
}
