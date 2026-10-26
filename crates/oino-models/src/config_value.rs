//! pi `resolve-config-value.ts`: API keys and header values that are literals, `$ENV` /
//! `${ENV}` templates, or `!command` output.
//!
//! - A value starting with `!` runs the rest as a shell command (`sh -c`, 10 s timeout) and uses
//!   its trimmed stdout; empty output, a timeout or a non-zero exit leaves it unresolved.
//!   [`resolve_config_value`] caches command results for the process lifetime (auth.json keys);
//!   [`resolve_config_value_uncached`] runs every time (models.json values).
//! - Otherwise `$NAME` / `${NAME}` interpolate environment variables; `$$` and `$!` escape a
//!   literal `$` / `!`. A template with an unset variable is unresolved.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::{Duration, Instant};

use indexmap::IndexMap;

use crate::env::{EnvSource, lookup};

/// Explicit variables consulted before the environment (a stored credential's `env`).
pub type ExplicitEnv = IndexMap<String, String>;

/// Timeout for `!command` values (pi: 10 000 ms).
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);

static COMMAND_CACHE: LazyLock<Mutex<HashMap<String, Option<String>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone, PartialEq, Eq)]
enum TemplatePart {
    Literal(String),
    Env(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Reference {
    /// Full config text including the leading `!`.
    Command(String),
    Template(Vec<TemplatePart>),
}

/// Failure to resolve a required config value (pi `resolveConfigValueOrThrow` messages).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigValueError {
    /// The `!command` produced no value.
    #[error("Failed to resolve {description} from shell command: {command}")]
    Command {
        /// What was being resolved, e.g. `API key for provider "x"`.
        description: String,
        /// Command text without the `!`.
        command: String,
    },
    /// One template variable is unset.
    #[error("Failed to resolve {description} from environment variable: {name}")]
    EnvVar {
        /// What was being resolved.
        description: String,
        /// The unset variable.
        name: String,
    },
    /// Several template variables are unset.
    #[error("Failed to resolve {description} from environment variables: {}", names.join(", "))]
    EnvVars {
        /// What was being resolved.
        description: String,
        /// The unset variables, in first-use order.
        names: Vec<String>,
    },
    /// Unresolved for another reason.
    #[error("Failed to resolve {description}")]
    Other {
        /// What was being resolved.
        description: String,
    },
}

fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn env_name_prefix(text: &str) -> &str {
    let mut end = 0;
    for (index, c) in text.char_indices() {
        let valid = if index == 0 {
            c.is_ascii_alphabetic() || c == '_'
        } else {
            c.is_ascii_alphanumeric() || c == '_'
        };
        if !valid {
            break;
        }
        end = index + c.len_utf8();
    }
    &text[..end]
}

fn push_literal(parts: &mut Vec<TemplatePart>, value: &str) {
    if value.is_empty() {
        return;
    }
    if let Some(TemplatePart::Literal(previous)) = parts.last_mut() {
        previous.push_str(value);
    } else {
        parts.push(TemplatePart::Literal(value.to_owned()));
    }
}

fn parse_template(config: &str) -> Vec<TemplatePart> {
    let mut parts = Vec::new();
    let mut index = 0;
    while index < config.len() {
        let Some(offset) = config[index..].find('$') else {
            push_literal(&mut parts, &config[index..]);
            break;
        };
        let dollar = index + offset;
        push_literal(&mut parts, &config[index..dollar]);
        let rest = &config[dollar + 1..];
        match rest.chars().next() {
            Some(next @ ('$' | '!')) => {
                push_literal(&mut parts, &next.to_string());
                index = dollar + 2;
            }
            Some('{') => {
                if let Some(close) = config[dollar + 2..].find('}') {
                    let end = dollar + 2 + close;
                    let name = &config[dollar + 2..end];
                    if is_env_name(name) {
                        parts.push(TemplatePart::Env(name.to_owned()));
                    } else {
                        push_literal(&mut parts, &config[dollar..=end]);
                    }
                    index = end + 1;
                } else {
                    push_literal(&mut parts, "$");
                    index = dollar + 1;
                }
            }
            _ => {
                let name = env_name_prefix(rest);
                if name.is_empty() {
                    push_literal(&mut parts, "$");
                    index = dollar + 1;
                } else {
                    parts.push(TemplatePart::Env(name.to_owned()));
                    index = dollar + 1 + name.len();
                }
            }
        }
    }
    parts
}

fn parse_reference(config: &str) -> Reference {
    if config.starts_with('!') {
        Reference::Command(config.to_owned())
    } else {
        Reference::Template(parse_template(config))
    }
}

fn template_env_names(parts: &[TemplatePart]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for part in parts {
        if let TemplatePart::Env(name) = part
            && !names.contains(name)
        {
            names.push(name.clone());
        }
    }
    names
}

fn resolve_template(
    parts: &[TemplatePart],
    explicit: Option<&ExplicitEnv>,
    env: &dyn EnvSource,
) -> Option<String> {
    let mut resolved = String::new();
    for part in parts {
        match part {
            TemplatePart::Literal(value) => resolved.push_str(value),
            TemplatePart::Env(name) => resolved.push_str(&lookup(name, explicit, env)?),
        }
    }
    Some(resolved)
}

/// pi `getConfigValueEnvVarName`: the variable when the value is exactly one `$NAME` reference.
#[must_use]
pub fn config_value_env_var_name(config: &str) -> Option<String> {
    match parse_reference(config) {
        Reference::Template(parts) => match parts.as_slice() {
            [TemplatePart::Env(name)] => Some(name.clone()),
            _ => None,
        },
        Reference::Command(_) => None,
    }
}

/// pi `getConfigValueEnvVarNames`: every distinct variable a template references.
#[must_use]
pub fn config_value_env_var_names(config: &str) -> Vec<String> {
    match parse_reference(config) {
        Reference::Template(parts) => template_env_names(&parts),
        Reference::Command(_) => Vec::new(),
    }
}

/// pi `getMissingConfigValueEnvVarNames`.
#[must_use]
pub fn missing_config_value_env_var_names(
    config: &str,
    explicit: Option<&ExplicitEnv>,
    env: &dyn EnvSource,
) -> Vec<String> {
    config_value_env_var_names(config)
        .into_iter()
        .filter(|name| lookup(name, explicit, env).is_none())
        .collect()
}

/// pi `isCommandConfigValue`.
#[must_use]
pub fn is_command_config_value(config: &str) -> bool {
    config.starts_with('!')
}

/// pi `isConfigValueConfigured`: every referenced variable is set (commands count as configured).
#[must_use]
pub fn is_config_value_configured(
    config: &str,
    explicit: Option<&ExplicitEnv>,
    env: &dyn EnvSource,
) -> bool {
    missing_config_value_env_var_names(config, explicit, env).is_empty()
}

/// pi `resolveConfigValue`: resolves a value; `!command` results are cached per process.
#[must_use]
pub fn resolve_config_value(
    config: &str,
    explicit: Option<&ExplicitEnv>,
    env: &dyn EnvSource,
) -> Option<String> {
    match parse_reference(config) {
        Reference::Command(command) => execute_cached(&command),
        Reference::Template(parts) => resolve_template(&parts, explicit, env),
    }
}

/// pi `resolveConfigValueUncached`: like [`resolve_config_value`] but runs commands every time.
#[must_use]
pub fn resolve_config_value_uncached(
    config: &str,
    explicit: Option<&ExplicitEnv>,
    env: &dyn EnvSource,
) -> Option<String> {
    match parse_reference(config) {
        Reference::Command(command) => execute_command(&command[1..]),
        Reference::Template(parts) => resolve_template(&parts, explicit, env),
    }
}

/// pi `resolveConfigValueOrThrow` (uncached).
///
/// # Errors
///
/// [`ConfigValueError`] naming the command or the unset variables.
pub fn resolve_config_value_strict(
    config: &str,
    description: &str,
    explicit: Option<&ExplicitEnv>,
    env: &dyn EnvSource,
) -> Result<String, ConfigValueError> {
    if let Some(value) = resolve_config_value_uncached(config, explicit, env) {
        return Ok(value);
    }
    let description = description.to_owned();
    if let Some(command) = config.strip_prefix('!') {
        return Err(ConfigValueError::Command {
            description,
            command: command.to_owned(),
        });
    }
    let mut missing = missing_config_value_env_var_names(config, explicit, env);
    match missing.len() {
        0 => Err(ConfigValueError::Other { description }),
        1 => Err(ConfigValueError::EnvVar {
            description,
            name: missing.remove(0),
        }),
        _ => Err(ConfigValueError::EnvVars {
            description,
            names: missing,
        }),
    }
}

/// pi `resolveHeaders`: resolves header values (cached commands); unresolved or empty values
/// are dropped and an empty result is `None`.
#[must_use]
pub fn resolve_headers(
    headers: Option<&IndexMap<String, String>>,
    explicit: Option<&ExplicitEnv>,
    env: &dyn EnvSource,
) -> Option<IndexMap<String, String>> {
    let resolved: IndexMap<String, String> = headers?
        .iter()
        .filter_map(|(key, value)| {
            resolve_config_value(value, explicit, env)
                .filter(|v| !v.is_empty())
                .map(|v| (key.clone(), v))
        })
        .collect();
    (!resolved.is_empty()).then_some(resolved)
}

/// pi `resolveHeadersOrThrow`: every header must resolve; `description` is extended with
/// ` header "<name>"`.
///
/// # Errors
///
/// The first header that fails to resolve.
pub fn resolve_headers_strict(
    headers: Option<&IndexMap<String, String>>,
    description: &str,
    explicit: Option<&ExplicitEnv>,
    env: &dyn EnvSource,
) -> Result<Option<IndexMap<String, String>>, ConfigValueError> {
    let Some(headers) = headers else {
        return Ok(None);
    };
    let mut resolved = IndexMap::new();
    for (key, value) in headers {
        let header_description = format!("{description} header \"{key}\"");
        resolved.insert(
            key.clone(),
            resolve_config_value_strict(value, &header_description, explicit, env)?,
        );
    }
    Ok((!resolved.is_empty()).then_some(resolved))
}

/// pi `clearConfigValueCache`: forgets cached `!command` results.
pub fn clear_config_value_cache() {
    COMMAND_CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clear();
}

fn execute_cached(command_config: &str) -> Option<String> {
    if let Some(cached) = COMMAND_CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(command_config)
    {
        return cached.clone();
    }
    let result = execute_command(&command_config[1..]);
    COMMAND_CACHE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(command_config.to_owned(), result.clone());
    result
}

fn shell_command(command: &str) -> Command {
    if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    } else {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(command);
        cmd
    }
}

/// Runs `command` through the default shell with a 10 s timeout; trimmed non-empty stdout of a
/// successful run, else `None` (pi `executeWithDefaultShell`).
fn execute_command(command: &str) -> Option<String> {
    let mut child = shell_command(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buffer = Vec::new();
        stdout.read_to_end(&mut buffer).map(|_| buffer)
    });
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(5));
            }
            Ok(None) | Err(_) => {
                // Timeout or wait failure: kill and report unresolved. Errors here mean the
                // child already exited, so there is nothing else to clean up.
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let output = reader.join().ok()?.ok()?;
    if !status?.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output).trim().to_owned();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;

    fn env() -> MapEnv {
        MapEnv::new()
            .with("FOO", "foo")
            .with("BAR", "bar")
            .with("EMPTY", "")
    }

    #[test]
    fn templates() {
        let env = env();
        let r = |c: &str| resolve_config_value(c, None, &env);
        assert_eq!(r("literal"), Some("literal".into()));
        assert_eq!(r("$FOO"), Some("foo".into()));
        assert_eq!(r("${FOO}-$BAR!"), Some("foo-bar!".into()));
        assert_eq!(r("$$FOO"), Some("$FOO".into()));
        assert_eq!(r("$!x"), Some("!x".into()));
        assert_eq!(r("$MISSING"), None);
        assert_eq!(r("$EMPTY"), None);
        assert_eq!(r("${not valid}"), Some("${not valid}".into()));
        assert_eq!(r("${FOO"), Some("${FOO".into()));
        assert_eq!(r("a$"), Some("a$".into()));
        assert_eq!(r("$1"), Some("$1".into()));
        assert_eq!(r(""), Some(String::new()));
    }

    #[test]
    fn explicit_env_wins() {
        let env = env();
        let explicit: ExplicitEnv = [("FOO".to_owned(), "override".to_owned())].into();
        assert_eq!(
            resolve_config_value("$FOO", Some(&explicit), &env),
            Some("override".into())
        );
    }

    #[test]
    fn env_names() {
        assert_eq!(config_value_env_var_name("$FOO"), Some("FOO".into()));
        assert_eq!(config_value_env_var_name("${FOO}"), Some("FOO".into()));
        assert_eq!(config_value_env_var_name("x$FOO"), None);
        assert_eq!(
            config_value_env_var_names("$A-${B}-$A"),
            vec!["A".to_owned(), "B".to_owned()]
        );
        assert_eq!(config_value_env_var_names("!echo $A"), Vec::<String>::new());
    }

    #[test]
    fn strict_errors() {
        let env = env();
        let err = resolve_config_value_strict("$A$B", "API key", None, &env).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Failed to resolve API key from environment variables: A, B"
        );
        let err = resolve_config_value_strict("${A}", "API key", None, &env).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Failed to resolve API key from environment variable: A"
        );
    }

    #[cfg(unix)]
    #[test]
    fn commands() {
        let env = env();
        assert_eq!(
            resolve_config_value_uncached("!printf ' hi \\n'", None, &env),
            Some("hi".into())
        );
        assert_eq!(resolve_config_value_uncached("!exit 3", None, &env), None);
        assert_eq!(resolve_config_value_uncached("!true", None, &env), None);
        let err = resolve_config_value_strict("!false", "API key", None, &env).unwrap_err();
        assert_eq!(
            err.to_string(),
            "Failed to resolve API key from shell command: false"
        );
    }
}
