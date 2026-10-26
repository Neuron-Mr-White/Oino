//! Shell resolution, environment and output sanitising (pi `utils/shell.ts`, `utils/ansi.ts`).

use std::sync::LazyLock;

use regex::Regex;

use crate::config::{SESSION_ENV_NAMES, SessionEnv};
use crate::env::{ExecutionEnv, path_exists};
use crate::node_path;

/// How the command reaches the shell.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandTransport {
    /// Appended as the last argument (`bash -c <command>`).
    Argv,
    /// Written to stdin (legacy WSL `bash.exe -s`).
    Stdin,
}

/// pi `ShellConfig`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellConfig {
    /// Shell executable.
    pub shell: String,
    /// Arguments before the command.
    pub args: Vec<String>,
    /// Command transport.
    pub transport: CommandTransport,
}

impl ShellConfig {
    fn argv(shell: &str, args: &[&str]) -> Self {
        Self {
            shell: shell.to_owned(),
            args: args.iter().map(|arg| (*arg).to_owned()).collect(),
            transport: CommandTransport::Argv,
        }
    }
}

/// pi `isLegacyWslBashPath`.
fn is_legacy_wsl_bash(path: &str) -> bool {
    if !cfg!(windows) {
        return false;
    }
    let normalized = path.replace('/', "\\").to_lowercase();
    if let Ok(root) = std::env::var("SystemRoot") {
        let bytes = root.as_bytes();
        let drive_like = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && (bytes[2] == b'\\' || bytes[2] == b'/');
        if !drive_like {
            return false;
        }
        let root = root
            .trim_end_matches(['\\', '/'])
            .replace('/', "\\")
            .to_lowercase();
        return ["system32", "sysnative"]
            .iter()
            .any(|dir| normalized == format!("{root}\\{dir}\\bash.exe"));
    }
    let rest = normalized.get(1..).unwrap_or("");
    normalized
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic())
        && (rest == ":\\windows\\system32\\bash.exe" || rest == ":\\windows\\sysnative\\bash.exe")
}

fn bash_config(shell: &str) -> ShellConfig {
    if is_legacy_wsl_bash(shell) {
        ShellConfig {
            shell: shell.to_owned(),
            args: vec!["-s".to_owned()],
            transport: CommandTransport::Stdin,
        }
    } else {
        ShellConfig::argv(shell, &["-c"])
    }
}

/// pi `findExecutableOnPath` (`which` on Unix, `where` on Windows; first line of output).
async fn find_executable_on_path(env: &dyn ExecutionEnv, executable: &str) -> Option<String> {
    let finder = if cfg!(windows) { "where" } else { "which" };
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        tokio::process::Command::new(finder)
            .arg(executable)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let first = crate::js::trim(&stdout)
        .split('\n')
        .next()
        .map(|line| line.trim_end_matches('\r'))
        .filter(|line| !line.is_empty())?
        .to_owned();
    if cfg!(windows) && !path_exists(env, &first).await {
        return None;
    }
    Some(first)
}

/// pi `getShellConfig(shellPath)`: the explicit shell, else `/bin/bash`, `bash` on `PATH`,
/// `sh` (Unix) or Git Bash / `bash.exe` on `PATH` (Windows).
///
/// # Errors
/// pi's texts: `Custom shell path not found: {path}`, or the Windows "No bash shell found"
/// help.
pub async fn resolve_shell_config(
    env: &dyn ExecutionEnv,
    shell_path: Option<&str>,
) -> Result<ShellConfig, String> {
    if let Some(custom) = shell_path.filter(|path| !path.is_empty()) {
        if path_exists(env, custom).await {
            return Ok(bash_config(custom));
        }
        return Err(format!("Custom shell path not found: {custom}"));
    }
    if cfg!(windows) {
        let mut paths = Vec::new();
        if let Ok(program_files) = std::env::var("ProgramFiles") {
            paths.push(format!("{program_files}\\Git\\bin\\bash.exe"));
        }
        if let Ok(program_files_x86) = std::env::var("ProgramFiles(x86)") {
            paths.push(format!("{program_files_x86}\\Git\\bin\\bash.exe"));
        }
        for path in &paths {
            if path_exists(env, path).await {
                return Ok(bash_config(path));
            }
        }
        if let Some(found) = find_executable_on_path(env, "bash.exe").await {
            return Ok(bash_config(&found));
        }
        let searched: Vec<String> = paths.iter().map(|path| format!("  {path}")).collect();
        return Err(format!(
            "No bash shell found. Options:\n  1. Install Git for Windows: https://git-scm.com/download/win\n  2. Add your bash to PATH (Cygwin, MSYS2, etc.)\n  3. Set shellPath in settings.json\n\nSearched Git Bash in:\n{}",
            searched.join("\n")
        ));
    }
    if path_exists(env, "/bin/bash").await {
        return Ok(bash_config("/bin/bash"));
    }
    if let Some(found) = find_executable_on_path(env, "bash").await {
        return Ok(bash_config(&found));
    }
    Ok(ShellConfig::argv("sh", &["-c"]))
}

/// pi `POWERSHELL_ARGS`.
pub const POWERSHELL_ARGS: [&str; 5] = [
    "-NoProfile",
    "-NonInteractive",
    "-ExecutionPolicy",
    "Bypass",
    "-Command",
];

/// pi `getPowerShellConfig`: `pwsh.exe` on `PATH`, PowerShell 7, `powershell.exe` on `PATH`,
/// then Windows PowerShell.
///
/// # Errors
/// pi's texts when not on Windows or when no PowerShell is installed.
pub async fn resolve_powershell_config(env: &dyn ExecutionEnv) -> Result<ShellConfig, String> {
    if !cfg!(windows) {
        return Err("The powershell tool is only available on Windows.".to_owned());
    }
    let mut shell = find_executable_on_path(env, "pwsh.exe").await;
    if shell.is_none()
        && let Ok(program_files) = std::env::var("ProgramFiles")
    {
        let candidate = format!("{program_files}\\PowerShell\\7\\pwsh.exe");
        if path_exists(env, &candidate).await {
            shell = Some(candidate);
        }
    }
    if shell.is_none() {
        shell = find_executable_on_path(env, "powershell.exe").await;
    }
    if shell.is_none()
        && let Ok(root) = std::env::var("SystemRoot")
    {
        let candidate = format!("{root}\\System32\\WindowsPowerShell\\v1.0\\powershell.exe");
        if path_exists(env, &candidate).await {
            shell = Some(candidate);
        }
    }
    let shell = shell.ok_or_else(|| {
        "No PowerShell executable found. Install PowerShell or add powershell.exe/pwsh.exe to PATH."
            .to_owned()
    })?;
    Ok(ShellConfig::argv(&shell, &POWERSHELL_ARGS))
}

/// pi `getShellEnv`: the base environment with `<agent_dir>/bin` prepended to `PATH` (once).
#[must_use]
pub fn shell_env(base: Option<&[(String, String)]>, agent_dir: &str) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = match base {
        Some(base) => base.to_vec(),
        None => std::env::vars().collect(),
    };
    let bin_dir = node_path::join(agent_dir, "bin");
    let delimiter = if cfg!(windows) { ';' } else { ':' };
    let key = env
        .iter()
        .map(|(key, _)| key.clone())
        .find(|key| key.eq_ignore_ascii_case("path"))
        .unwrap_or_else(|| "PATH".to_owned());
    let current = env
        .iter()
        .find(|(name, _)| *name == key)
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    let has_bin = current.split(delimiter).any(|entry| entry == bin_dir);
    let updated = if has_bin {
        current
    } else if current.is_empty() {
        bin_dir
    } else {
        format!("{bin_dir}{delimiter}{current}")
    };
    env.retain(|(name, _)| *name != key);
    env.push((key, updated));
    env
}

/// The shell tool environment (pi `resolveSpawnContext`, D12): inherited session variables
/// removed, then `session` exported under `OINO_*` and `PI_*`.
#[must_use]
pub fn tool_env(
    base: Vec<(String, String)>,
    session: Option<&SessionEnv>,
) -> Vec<(String, String)> {
    let mut env: Vec<(String, String)> = base
        .into_iter()
        .filter(|(name, _)| !SESSION_ENV_NAMES.contains(&name.as_str()))
        .collect();
    if let Some(session) = session {
        env.extend(session.vars());
    }
    env
}

/// pi `sanitizeBinaryOutput`: drops C0 controls except tab/LF/CR, and U+FFF9..U+FFFB.
#[must_use]
pub fn sanitize_binary_output(text: &str) -> String {
    text.chars()
        .filter(|ch| {
            !matches!(ch, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fff9}'..='\u{fffb}')
        })
        .collect()
}

static ANSI: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(
        r"(?:\x1B\](?s:.)*?(?:\x07|\x1B\x5C|\u{9C}))|[\x1B\u{9B}][\[\]()#;?]*(?:[0-9]{1,4}(?:[;:][0-9]{0,4})*)?[0-9A-PR-TZcf-nq-uy=><~]",
    )
    .ok()
});

static CSI_UNFINISHED: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"^[\x1B\u{9B}][\[\]()#;?]*(?:[0-9]{1,4}(?:[;:][0-9]{0,4})*)?$").ok()
});

/// pi `stripAnsi`.
#[must_use]
pub fn strip_ansi(text: &str) -> String {
    if !text.contains(['\u{1b}', '\u{9b}']) {
        return text.to_owned();
    }
    match ANSI.as_ref() {
        Some(regex) => regex.replace_all(text, "").into_owned(),
        None => text.to_owned(),
    }
}

/// Whether `rest` is an OSC sequence without its terminator.
fn is_unfinished_osc(rest: &str) -> bool {
    let Some(body) = rest.strip_prefix("\u{1b}]") else {
        return false;
    };
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\u{7}' | '\u{9c}' => return false,
            '\u{1b}' if chars.peek() == Some(&'\\') => return false,
            _ => {}
        }
    }
    true
}

/// Longest unfinished sequence held back while streaming (pi `MAX_PENDING_ANSI_LENGTH`).
const MAX_PENDING_ANSI_LENGTH: usize = 256;

/// pi `splitIncompleteAnsiSuffix`: `(complete, pending)` where `pending` is a trailing
/// unfinished escape sequence to prepend to the next chunk.
#[must_use]
pub fn split_incomplete_ansi_suffix(text: &str) -> (String, String) {
    if !text.contains(['\u{1b}', '\u{9b}']) {
        return (text.to_owned(), String::new());
    }
    let total_units = crate::js::utf16_len(text);
    let window_units = total_units.saturating_sub(MAX_PENDING_ANSI_LENGTH);
    let mut units = 0;
    for (index, ch) in text.char_indices() {
        if units >= window_units && (ch == '\u{1b}' || ch == '\u{9b}') {
            let rest = &text[index..];
            let csi = CSI_UNFINISHED
                .as_ref()
                .is_some_and(|regex| regex.is_match(rest));
            if is_unfinished_osc(rest) || csi {
                return (text[..index].to_owned(), rest.to_owned());
            }
        }
        units += ch.len_utf16();
    }
    (text.to_owned(), String::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi_like_pi() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}x"), "x");
        assert_eq!(strip_ansi("\u{1b}[12345m"), "m");
    }

    #[test]
    fn splits_unfinished_sequences() {
        assert_eq!(
            split_incomplete_ansi_suffix("ab\u{1b}[3"),
            ("ab".to_owned(), "\u{1b}[3".to_owned())
        );
        assert_eq!(
            split_incomplete_ansi_suffix("ab\u{1b}]0;t"),
            ("ab".to_owned(), "\u{1b}]0;t".to_owned())
        );
        assert_eq!(
            split_incomplete_ansi_suffix("ab\u{1b}[3m"),
            ("ab\u{1b}[3m".to_owned(), String::new())
        );
    }

    #[test]
    fn sanitizes_controls() {
        assert_eq!(sanitize_binary_output("a\u{0}b\tc\u{1b}\n"), "ab\tc\n");
    }

    #[test]
    fn env_prepends_bin_dir_and_exports_session() {
        let base = vec![
            ("PATH".to_owned(), "/usr/bin".to_owned()),
            ("PI_MODEL".to_owned(), "old".to_owned()),
        ];
        let env = shell_env(Some(&base), "/agent");
        assert!(env.contains(&("PATH".to_owned(), "/agent/bin:/usr/bin".to_owned())));
        let session = SessionEnv {
            session_id: "s".into(),
            ..SessionEnv::default()
        };
        let env = tool_env(env, Some(&session));
        assert!(!env.iter().any(|(k, v)| k == "PI_MODEL" && v == "old"));
        assert!(env.contains(&("OINO_SESSION_ID".to_owned(), "s".to_owned())));
        assert!(env.contains(&("PI_SESSION_ID".to_owned(), "s".to_owned())));
    }
}
