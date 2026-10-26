//! Prompt templates (pi `core/prompt-templates.ts`): loading, argument parsing and
//! substitution, `/name args` expansion.

use std::fs;
use std::path::{Path, PathBuf};

use oino_settings::paths::{is_under_path, project_dir, resolve_path};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::diagnostics::ResourceDiagnostic;
use crate::frontmatter::parse_frontmatter;
use crate::source::{SourceInfo, SourceScope};
use crate::walk::{has_suffix, read_dir_sorted};

/// A prompt template loaded from a markdown file (pi `PromptTemplate`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PromptTemplate {
    /// File name without `.md`; invoked as `/name`.
    pub name: String,
    /// Frontmatter `description`, else the first non-empty body line (60 chars + `...`).
    pub description: String,
    /// Frontmatter `argument-hint`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    /// Template body (frontmatter removed).
    pub content: String,
    /// Provenance.
    pub source_info: SourceInfo,
    /// Absolute template path.
    pub file_path: PathBuf,
}

/// Templates plus diagnostics (pi `LoadPromptTemplatesResult`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoadPromptTemplatesResult {
    /// Loaded templates (not deduplicated; see the resource loader).
    pub templates: Vec<PromptTemplate>,
    /// Read/parse diagnostics.
    pub diagnostics: Vec<ResourceDiagnostic>,
}

/// pi `parseCommandArgs`: split on whitespace honouring `'…'` and `"…"` quotes (no escapes;
/// quoted empty strings are kept).
#[must_use]
pub fn parse_command_args(args: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut quote: Option<char> = None;
    for c in args.chars() {
        if let Some(open) = quote {
            if c == open {
                quote = None;
            } else {
                current.push(c);
            }
        } else if c == '"' || c == '\'' {
            quote = Some(c);
            has_token = true;
        } else if is_js_whitespace(c) {
            if has_token {
                result.push(std::mem::take(&mut current));
                has_token = false;
            }
        } else {
            current.push(c);
            has_token = true;
        }
    }
    if has_token {
        result.push(current);
    }
    result
}

/// JavaScript `/\s/` (Unicode `White_Space` plus BOM).
fn is_js_whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{feff}'
}

/// pi `substituteArgs`: `$1…`, `$@`, `$ARGUMENTS`, `${N:-default}`, `${@:-default}`,
/// `${ARGUMENTS:-default}`, `${@:N}`, `${@:N:L}`. Replacement is single-pass (substituted values
/// are not re-expanded).
#[must_use]
pub fn substitute_args(content: &str, args: &[String]) -> String {
    let all = args.join(" ");
    let mut out = String::with_capacity(content.len());
    let mut rest = content;
    while let Some(dollar) = rest.find('$') {
        out.push_str(&rest[..dollar]);
        let tail = &rest[dollar..];
        if let Some((consumed, replacement)) = match_placeholder(tail, args, &all) {
            out.push_str(&replacement);
            rest = &tail[consumed..];
        } else {
            out.push('$');
            rest = &tail[1..];
        }
    }
    out.push_str(rest);
    out
}

/// Try the pi regex alternatives at `tail` (which starts with `$`), in order; returns the
/// consumed length and the replacement.
fn match_placeholder(tail: &str, args: &[String], all: &str) -> Option<(usize, String)> {
    let body = &tail[1..];
    if let Some(inner) = body.strip_prefix('{') {
        // \$\{(\d+|ARGUMENTS|@):-([^}]*)\}
        let target_len = leading_target(inner);
        if target_len > 0
            && let Some(after) = inner[target_len..].strip_prefix(":-")
            && let Some(close) = after.find('}')
        {
            let target = &inner[..target_len];
            let default = &after[..close];
            let value = if target == "@" || target == "ARGUMENTS" {
                all.to_owned()
            } else {
                positional(args, target).unwrap_or_default()
            };
            let consumed = 2 + target_len + 2 + close + 1;
            return Some((
                consumed,
                if value.is_empty() {
                    default.to_owned()
                } else {
                    value
                },
            ));
        }
        // \$\{@:(\d+)(?::(\d+))?\}
        if let Some(slice) = inner.strip_prefix("@:") {
            let start_len = digits(slice);
            if start_len > 0 {
                let start_text = &slice[..start_len];
                let after_start = &slice[start_len..];
                let (length, after) = match after_start.strip_prefix(':') {
                    Some(len_text) if digits(len_text) > 0 => {
                        let n = digits(len_text);
                        (Some(&len_text[..n]), &len_text[n..])
                    }
                    _ => (None, after_start),
                };
                if after.starts_with('}') {
                    let consumed = tail.len() - after.len() + 1;
                    let start = parse_index(start_text).saturating_sub(1);
                    let selected: Vec<&str> = args
                        .iter()
                        .skip(start)
                        .take(length.map_or(usize::MAX, parse_index))
                        .map(String::as_str)
                        .collect();
                    return Some((consumed, selected.join(" ")));
                }
            }
        }
        return None;
    }
    // \$(ARGUMENTS|@|\d+)
    if body.starts_with("ARGUMENTS") {
        return Some((1 + "ARGUMENTS".len(), all.to_owned()));
    }
    if body.starts_with('@') {
        return Some((2, all.to_owned()));
    }
    let n = digits(body);
    if n > 0 {
        return Some((1 + n, positional(args, &body[..n]).unwrap_or_default()));
    }
    None
}

/// Length of a leading `\d+`, `ARGUMENTS` or `@` target.
fn leading_target(text: &str) -> usize {
    if text.starts_with("ARGUMENTS") {
        return "ARGUMENTS".len();
    }
    if text.starts_with('@') {
        return 1;
    }
    digits(text)
}

fn digits(text: &str) -> usize {
    text.bytes().take_while(u8::is_ascii_digit).count()
}

/// Saturating decimal parse (huge indices simply select nothing).
fn parse_index(text: &str) -> usize {
    text.bytes().fold(0usize, |acc, b| {
        acc.saturating_mul(10).saturating_add(usize::from(b - b'0'))
    })
}

/// `args[N - 1]` for a 1-based decimal `N` (`$0` and out-of-range are empty).
fn positional(args: &[String], number: &str) -> Option<String> {
    parse_index(number)
        .checked_sub(1)
        .and_then(|index| args.get(index))
        .cloned()
}

/// pi `loadTemplateFromFile`.
fn load_template_from_file(
    file_path: &Path,
    source_info: SourceInfo,
) -> (Option<PromptTemplate>, Vec<ResourceDiagnostic>) {
    let raw = match fs::read_to_string(file_path) {
        Ok(raw) => raw,
        Err(error) => {
            return (
                None,
                vec![ResourceDiagnostic::warning(error.to_string(), file_path)],
            );
        }
    };
    let parsed = match parse_frontmatter(&raw) {
        Ok(parsed) => parsed,
        Err(error) => {
            return (
                None,
                vec![ResourceDiagnostic::warning(error.message, file_path)],
            );
        }
    };
    let name = file_path
        .file_name()
        .map(|name| name.to_string_lossy())
        .map(|name| name.strip_suffix(".md").unwrap_or(&name).to_owned())
        .unwrap_or_default();
    let mut description = parsed
        .fields
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    if description.is_empty()
        && let Some(first) = parsed.body.split('\n').find(|line| !line.trim().is_empty())
    {
        description = js_slice(first, 60);
        if first.encode_utf16().count() > 60 {
            description.push_str("...");
        }
    }
    let argument_hint = parsed
        .fields
        .get("argument-hint")
        .and_then(Value::as_str)
        .filter(|hint| !hint.is_empty())
        .map(str::to_owned);
    let template = PromptTemplate {
        name,
        description,
        argument_hint,
        content: parsed.body,
        source_info,
        file_path: file_path.to_path_buf(),
    };
    (Some(template), Vec::new())
}

/// JavaScript `text.slice(0, n)` over UTF-16 code units (never splitting a char).
fn js_slice(text: &str, n: usize) -> String {
    let mut units = 0;
    let mut out = String::new();
    for c in text.chars() {
        units += c.len_utf16();
        if units > n {
            break;
        }
        out.push(c);
    }
    out
}

/// pi `loadTemplatesFromDir`: direct `.md` children (non-recursive, symlinks followed).
fn load_templates_from_dir(
    dir: &Path,
    source_info: &dyn Fn(&Path) -> SourceInfo,
) -> LoadPromptTemplatesResult {
    let mut result = LoadPromptTemplatesResult::default();
    let Some(entries) = read_dir_sorted(dir) else {
        return result;
    };
    for entry in entries {
        if entry.is_file && has_suffix(&entry.name, ".md") {
            let (template, diagnostics) =
                load_template_from_file(&entry.path, source_info(&entry.path));
            result.templates.extend(template);
            result.diagnostics.extend(diagnostics);
        }
    }
    result
}

/// Inputs of [`load_prompt_templates`] (pi `LoadPromptTemplatesOptions`).
#[derive(Clone, Debug)]
pub struct LoadPromptTemplatesOptions<'a> {
    /// Working directory.
    pub cwd: &'a Path,
    /// Agent dir.
    pub agent_dir: &'a Path,
    /// Home dir for `~` expansion.
    pub home: Option<&'a Path>,
    /// Explicit template files or directories (missing paths are skipped silently).
    pub prompt_paths: &'a [PathBuf],
    /// Also load `<agent>/prompts` and `<cwd>/.oino/prompts` first.
    pub include_defaults: bool,
}

/// pi `loadPromptTemplates`.
#[must_use]
pub fn load_prompt_templates(
    options: &LoadPromptTemplatesOptions<'_>,
) -> LoadPromptTemplatesResult {
    let global_dir = options.agent_dir.join("prompts");
    let project_prompts = project_dir(options.cwd).join("prompts");
    let source_info = |path: &Path| {
        if is_under_path(path, &global_dir) {
            SourceInfo::synthetic(
                path,
                "local",
                Some(SourceScope::User),
                Some(global_dir.clone()),
            )
        } else if is_under_path(path, &project_prompts) {
            SourceInfo::synthetic(
                path,
                "local",
                Some(SourceScope::Project),
                Some(project_prompts.clone()),
            )
        } else {
            let base = if path.is_dir() {
                path.to_path_buf()
            } else {
                path.parent().unwrap_or(path).to_path_buf()
            };
            SourceInfo::synthetic(path, "local", None, Some(base))
        }
    };
    let mut result = LoadPromptTemplatesResult::default();
    let mut add = |other: LoadPromptTemplatesResult| {
        result.templates.extend(other.templates);
        result.diagnostics.extend(other.diagnostics);
    };
    if options.include_defaults {
        add(load_templates_from_dir(&global_dir, &source_info));
        add(load_templates_from_dir(&project_prompts, &source_info));
    }
    for raw in options.prompt_paths {
        let path = resolve_path(&raw.to_string_lossy(), options.cwd, options.home, true);
        let Ok(metadata) = fs::metadata(&path) else {
            continue;
        };
        if metadata.is_dir() {
            add(load_templates_from_dir(&path, &source_info));
        } else if metadata.is_file() && has_suffix(&path.to_string_lossy(), ".md") {
            let (template, diagnostics) = load_template_from_file(&path, source_info(&path));
            add(LoadPromptTemplatesResult {
                templates: template.into_iter().collect(),
                diagnostics,
            });
        }
    }
    result
}

/// pi `expandPromptTemplate`: `/name args` → the substituted template; `None` when `text` is
/// not a known template invocation.
#[must_use]
pub fn expand_prompt_template(text: &str, templates: &[PromptTemplate]) -> Option<String> {
    let (name, args) = parse_slash_invocation(text)?;
    let template = templates.iter().find(|template| template.name == name)?;
    Some(substitute_args(
        &template.content,
        &parse_command_args(args),
    ))
}

/// Split `/name rest` per pi `^\/([^\s]+)(?:\s+([\s\S]*))?$` → `(name, rest)`.
#[must_use]
pub fn parse_slash_invocation(text: &str) -> Option<(&str, &str)> {
    let rest = text.strip_prefix('/')?;
    let name_end = rest.find(is_js_whitespace).unwrap_or(rest.len());
    if name_end == 0 {
        return None;
    }
    let name = &rest[..name_end];
    let args = rest[name_end..].trim_start_matches(is_js_whitespace);
    Some((name, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|&s| s.to_owned()).collect()
    }

    #[test]
    fn parses_quotes_like_pi() {
        assert_eq!(
            parse_command_args(r#"a "b c" 'd "e"' "" x"y"z"#),
            args(&["a", "b c", "d \"e\"", "", "xyz"])
        );
        assert_eq!(parse_command_args("  "), Vec::<String>::new());
        assert_eq!(
            parse_command_args("\"unterminated x"),
            args(&["unterminated x"])
        );
    }

    #[test]
    fn substitutes_like_pi() {
        let a = args(&["one", "two", "three"]);
        assert_eq!(
            substitute_args("$1-$2-$4|$@|$ARGUMENTS", &a),
            "one-two-|one two three|one two three"
        );
        assert_eq!(
            substitute_args("${2:-x} ${5:-dflt} ${@:2} ${@:2:1} ${@:0}", &a),
            "two dflt two three two one two three"
        );
        assert_eq!(substitute_args("${@:-none}", &[]), "none");
        assert_eq!(substitute_args("cost $ 5 $x ${y}", &a), "cost $ 5 $x ${y}");
        assert_eq!(substitute_args("$10", &a), "");
        assert_eq!(substitute_args("$1", &args(&["$2", "b"])), "$2");
    }

    #[test]
    fn slash_invocation() {
        assert_eq!(parse_slash_invocation("/t a b"), Some(("t", "a b")));
        assert_eq!(parse_slash_invocation("/t"), Some(("t", "")));
        assert_eq!(parse_slash_invocation("/ t"), None);
        assert_eq!(parse_slash_invocation("t"), None);
    }
}
