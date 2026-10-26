//! Response formatting helpers (unipi `src/render/parse.ts`, pure): shell commands split into
//! shell text and embedded code (heredoc bodies, `python -c "…"`, `node -e '…'`), pi's display
//! diff, exit codes and test summaries from tool output.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

/// A command segment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    /// Embedded code (vs shell text).
    pub code: bool,
    /// Text.
    pub text: String,
    /// Language.
    pub lang: String,
}

fn re(s: &str) -> Option<Regex> {
    Regex::new(s).ok()
}

static RUNTIME_LANG: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"\b(?:python3?|uv run python|ipython)\b", "python"),
        (r"\b(?:node|deno|bun|tsx|ts-node)\b", "javascript"),
        (
            r"\b(?:psql|sqlite3|mysql|duckdb|clickhouse-client)\b",
            "sql",
        ),
        (r"\b(?:ruby|irb)\b", "ruby"),
        (r"\bphp\b", "php"),
        (r"\bperl\b", "perl"),
        (r"\b(?:lua|luajit)\b", "lua"),
        (r"\b(?:bash|sh|zsh|ssh\s+\S+)\b", "bash"),
        (r"\bjq\b", "json"),
    ]
    .into_iter()
    .filter_map(|(p, l)| Some((re(p)?, l)))
    .collect()
});
static TARGET: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r#"(?:>|tee\s+(?:-a\s+)?)\s*["']?([^\s"'|;&]+)["']?\s*(?:<<|$)"#));
static HEREDOC: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r#"<<-?\s*(['"]?)([A-Za-z_][A-Za-z0-9_]*)(['"]?)[^\n]*\n"#));
static INLINE: LazyLock<Option<Regex>> = LazyLock::new(|| {
    re(
        r#"\b(python3?|node|deno|bun|ruby|perl|psql|sqlite3)\b([^\n'"]*?\s-(?:c|e|-eval|-command)\s+)(["'])"#,
    )
});
static DIFF_ROW: LazyLock<Option<Regex>> = LazyLock::new(|| re(r"^([+\- ])(\s*\d+) (.*)$"));
static EXIT: LazyLock<Option<Regex>> = LazyLock::new(|| re(r"Command exited with code (\d+)\s*$"));
static STOPPED: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r"Command (?:aborted|terminated without an exit code)\s*$"));
static STATUS: LazyLock<Option<Regex>> = LazyLock::new(|| {
    re(
        r"\n*(?:Command exited with code \d+|Command aborted|Command terminated without an exit code)\s*$",
    )
});
static ANSI: LazyLock<Option<Regex>> = LazyLock::new(|| re("\x1b\\[[0-9;]*m"));
static NODE_PASS: LazyLock<Option<Regex>> = LazyLock::new(|| re(r"^ℹ pass (\d+)"));
static NODE_FAIL: LazyLock<Option<Regex>> = LazyLock::new(|| re(r"^ℹ fail (\d+)"));
static VITEST: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r"(?i)Tests?:?\s+(?:(\d+) failed[,|\s]+)?(\d+) passed"));
static CARGO: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r"test result: \w+\. (\d+) passed; (\d+) failed"));
static PYTEST: LazyLock<Option<Regex>> =
    LazyLock::new(|| re(r"^=+ (?:(\d+) failed, )?(\d+) passed"));

/// Language by file extension (unipi `EXT_LANG` plus common source types).
#[must_use]
pub fn lang_for_ext(ext: &str) -> Option<&'static str> {
    Some(match ext.to_ascii_lowercase().as_str() {
        "py" => "python",
        "js" | "mjs" | "cjs" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "jsx" => "jsx",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "sql" => "sql",
        "sh" | "bash" | "zsh" => "bash",
        "rs" => "rust",
        "go" => "go",
        "rb" => "ruby",
        "md" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "xml" => "xml",
        "ini" | "conf" => "ini",
        "dockerfile" => "dockerfile",
        "c" | "h" => "c",
        "cpp" | "cc" | "hpp" => "cpp",
        "java" => "java",
        "kt" => "kotlin",
        "swift" => "swift",
        "lua" => "lua",
        "php" => "php",
        _ => return None,
    })
}

/// Language of a path.
#[must_use]
pub fn lang_for_path(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    if name.eq_ignore_ascii_case("dockerfile") {
        return Some("dockerfile");
    }
    lang_for_ext(name.rsplit_once('.')?.1)
}

/// Language of code fed to the command before a heredoc marker.
#[must_use]
pub fn lang_for_prefix(prefix: &str) -> String {
    let line = prefix.rsplit('\n').next().unwrap_or(prefix);
    if let Some(ext) = TARGET
        .as_ref()
        .and_then(|r| r.captures(line))
        .and_then(|c| c.get(1).map(|m| m.as_str().to_owned()))
        .and_then(|t| t.rsplit_once('.').map(|(_, e)| e.to_owned()))
        && let Some(l) = lang_for_ext(&ext)
    {
        return l.to_owned();
    }
    RUNTIME_LANG
        .iter()
        .find(|(r, _)| r.is_match(line))
        .map_or_else(|| "text".to_owned(), |(_, l)| (*l).to_owned())
}

fn push(out: &mut Vec<Segment>, code: bool, text: &str, lang: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = out.last_mut()
        && last.code == code
        && last.lang == lang
    {
        last.text.push_str(text);
        return;
    }
    out.push(Segment {
        code,
        text: text.to_owned(),
        lang: lang.to_owned(),
    });
}

fn push_inline(out: &mut Vec<Segment>, text: &str) {
    let mut at = 0;
    if let Some(inline) = INLINE.as_ref() {
        let mut search = 0;
        while let Some(c) = inline.captures_at(text, search) {
            let (Some(whole), Some(runtime), Some(quote)) = (c.get(0), c.get(1), c.get(3)) else {
                break;
            };
            let q = quote.as_str();
            let body_start = whole.end();
            let Some(close) = text[body_start..].find(q) else {
                search = whole.end();
                continue;
            };
            push(out, false, &text[at..body_start], "bash");
            push(
                out,
                true,
                &text[body_start..body_start + close],
                &lang_for_prefix(runtime.as_str()),
            );
            push(out, false, q, "bash");
            at = body_start + close + q.len();
            search = at;
        }
    }
    push(out, false, &text[at..], "bash");
}

/// Shell text and embedded code, in order.
#[must_use]
pub fn split_command(command: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut at = 0;
    if let Some(heredoc) = HEREDOC.as_ref() {
        let mut search = 0;
        while let Some(m) = heredoc.captures_at(command, search) {
            let (Some(whole), Some(open), Some(tag), Some(close_q)) =
                (m.get(0), m.get(1), m.get(2), m.get(3))
            else {
                break;
            };
            search = whole.end();
            if open.as_str() != close_q.as_str() {
                continue;
            }
            let body_start = whole.end();
            let rest = &command[body_start..];
            let Some(end) = re(&format!(
                r"(?m)^[\t ]*{}[\t ]*$",
                regex::escape(tag.as_str())
            )) else {
                continue;
            };
            let Some(close) = end.find(rest) else {
                continue;
            };
            push_inline(&mut out, &command[at..body_start]);
            push(
                &mut out,
                true,
                &rest[..close.start()],
                &lang_for_prefix(&command[..whole.start()]),
            );
            at = body_start + close.start();
            search = at;
        }
    }
    push_inline(&mut out, &command[at..]);
    out
}

/// One display-diff row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffRow {
    /// Added line.
    Add(Option<u64>, String),
    /// Removed line.
    Del(Option<u64>, String),
    /// Context line.
    Ctx(Option<u64>, String),
    /// `...` gap.
    Gap,
}

/// pi's display diff (`+12 text`, `-12 text`, ` 12 text`, `...`) → rows.
#[must_use]
pub fn parse_diff(diff: &str) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    for raw in diff.split('\n') {
        if let Some(c) = DIFF_ROW.as_ref().and_then(|r| r.captures(raw)) {
            let line = c[2].trim().parse().ok();
            let text = c[3].to_owned();
            rows.push(match &c[1] {
                "+" => DiffRow::Add(line, text),
                "-" => DiffRow::Del(line, text),
                _ => DiffRow::Ctx(line, text),
            });
        } else if raw.trim() == "..."
            || (raw.len() > 1
                && matches!(raw.as_bytes()[0], b'+' | b'-' | b' ')
                && raw[1..].trim() == "...")
        {
            rows.push(DiffRow::Gap);
        } else if !raw.is_empty() {
            rows.push(DiffRow::Ctx(None, raw.to_owned()));
        }
    }
    rows
}

/// `(added, removed)`.
#[must_use]
pub fn diff_stats(rows: &[DiffRow]) -> (usize, usize) {
    (
        rows.iter()
            .filter(|r| matches!(r, DiffRow::Add(..)))
            .count(),
        rows.iter()
            .filter(|r| matches!(r, DiffRow::Del(..)))
            .count(),
    )
}

/// The rows as a unified patch body (for OUS `diff` nodes).
#[must_use]
pub fn to_patch(rows: &[DiffRow]) -> String {
    rows.iter()
        .map(|r| match r {
            DiffRow::Add(_, t) => format!("+{t}"),
            DiffRow::Del(_, t) => format!("-{t}"),
            DiffRow::Ctx(_, t) => format!(" {t}"),
            DiffRow::Gap => "@@ … @@".to_owned(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Exit code from pi's bash error text; 0 for a success, `None` when stopped.
#[must_use]
pub fn exit_code(text: &str, is_error: bool) -> Option<i64> {
    if let Some(c) = EXIT.as_ref().and_then(|r| r.captures(text)) {
        return c[1].parse().ok();
    }
    if STOPPED.as_ref().is_some_and(|r| r.is_match(text)) {
        return None;
    }
    Some(i64::from(is_error))
}

/// Output without pi's trailing status line.
#[must_use]
pub fn strip_status(text: &str) -> String {
    STATUS
        .as_ref()
        .map_or_else(|| text.to_owned(), |r| r.replace(text, "").into_owned())
}

/// `(passed, failed)` from node:test, vitest/jest, cargo and pytest summary lines.
#[must_use]
pub fn test_summary(output: &str) -> Option<(u64, u64)> {
    let (mut passed, mut failed): (Option<u64>, Option<u64>) = (None, None);
    let num = |m: Option<regex::Match<'_>>| m.and_then(|m| m.as_str().parse::<u64>().ok());
    let mut add = |p: Option<u64>, f: Option<u64>| {
        if let Some(p) = p {
            passed = Some(passed.unwrap_or(0) + p);
        }
        if let Some(f) = f {
            failed = Some(failed.unwrap_or(0) + f);
        }
    };
    for line in output.split('\n') {
        let clean = ANSI
            .as_ref()
            .map_or_else(|| line.to_owned(), |r| r.replace_all(line, "").into_owned());
        let cap = |r: &Option<Regex>| r.as_ref().and_then(|r| r.captures(&clean));
        if let Some(c) = cap(&NODE_PASS) {
            add(num(c.get(1)), None);
        } else if let Some(c) = cap(&NODE_FAIL) {
            add(None, num(c.get(1)));
        } else if let Some(c) = cap(&VITEST) {
            add(num(c.get(2)), Some(num(c.get(1)).unwrap_or(0)));
        } else if let Some(c) = cap(&CARGO) {
            add(num(c.get(1)), num(c.get(2)));
        } else if let Some(c) = cap(&PYTEST) {
            add(num(c.get(2)), Some(num(c.get(1)).unwrap_or(0)));
        }
    }
    if passed.is_none() && failed.is_none() {
        None
    } else {
        Some((passed.unwrap_or(0), failed.unwrap_or(0)))
    }
}

/// Pretty JSON when the whole output is one JSON value.
#[must_use]
pub fn as_json(output: &str) -> Option<String> {
    let t = output.trim();
    if !(t.starts_with('{') || t.starts_with('[')) || t.len() > 200_000 {
        return None;
    }
    serde_json::to_string_pretty(&serde_json::from_str::<Value>(t).ok()?).ok()
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty,
    clippy::format_collect,
    clippy::cloned_ref_to_slice_refs,
    clippy::duration_suboptimal_units,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping"
)]
mod tests {
    use super::*;

    #[test]
    fn python_heredoc_and_inline_code() {
        let cmd = "cd /repo && grep -n x api.py &&\n  python - <<'PYEOF'\ns = open('a').read()\nprint(s)\nPYEOF\npython -c \"import api; print('ok')\"";
        let segs = split_command(cmd);
        let kinds: Vec<(bool, &str)> = segs.iter().map(|s| (s.code, s.lang.as_str())).collect();
        assert_eq!(
            kinds,
            [
                (false, "bash"),
                (true, "python"),
                (false, "bash"),
                (true, "python"),
                (false, "bash")
            ]
        );
        assert_eq!(segs[1].text, "s = open('a').read()\nprint(s)\n");
        assert_eq!(segs[3].text, "import api; print('ok')");
        assert_eq!(
            segs.iter().map(|s| s.text.as_str()).collect::<String>(),
            cmd
        );
    }

    #[test]
    fn redirect_target_extension_and_plain_commands() {
        assert_eq!(lang_for_prefix("cat > config.yaml"), "yaml");
        assert_eq!(
            split_command("cat > a.ts <<EOF\nconst x = 1;\nEOF")[1].lang,
            "typescript"
        );
        assert_eq!(split_command("ls -la && git status").len(), 1);
        assert_eq!(lang_for_prefix("node"), "javascript");
        assert_eq!(lang_for_prefix("echo hi"), "text");
        let mismatched = split_command("cat <<'EOF\"\nx\nEOF");
        assert_eq!(mismatched.len(), 1, "mismatched quotes are not a heredoc");
        assert_eq!(split_command("python -c 'unterminated").len(), 1);
    }

    #[test]
    fn display_diff_and_status() {
        let rows = parse_diff(
            " 206     # comment\n+208     if x:\n+209         raise\n-210     old\n   ...\n 212 tail",
        );
        assert_eq!(rows[0], DiffRow::Ctx(Some(206), "    # comment".into()));
        assert!(matches!(rows[1], DiffRow::Add(Some(208), _)));
        assert!(matches!(rows[3], DiffRow::Del(Some(210), _)));
        assert_eq!(rows[4], DiffRow::Gap);
        assert!(matches!(rows[5], DiffRow::Ctx(Some(212), _)));
        assert_eq!(diff_stats(&rows), (2, 1));
        assert!(to_patch(&rows).contains("\n+    if x:\n"));
        assert_eq!(
            exit_code("boom\n\nCommand exited with code 2", true),
            Some(2)
        );
        assert_eq!(exit_code("ok", false), Some(0));
        assert_eq!(exit_code("x", true), Some(1));
        assert_eq!(exit_code("x\nCommand aborted", true), None);
        assert_eq!(strip_status("out\n\nCommand exited with code 2"), "out");
    }

    #[test]
    fn test_summaries_and_json() {
        assert_eq!(test_summary("ℹ pass 83\nℹ fail 0"), Some((83, 0)));
        assert_eq!(
            test_summary("      Tests  1 failed | 37 passed (38)"),
            Some((37, 1))
        );
        assert_eq!(
            test_summary("test result: ok. 12 passed; 0 failed; 0 ignored"),
            Some((12, 0))
        );
        assert_eq!(
            test_summary("===== 2 failed, 5 passed in 1s ====="),
            Some((5, 2))
        );
        assert_eq!(test_summary("\x1b[32mℹ pass 3\x1b[0m"), Some((3, 0)));
        assert_eq!(test_summary("hello"), None);
        assert_eq!(as_json("{\"a\":1}").as_deref(), Some("{\n  \"a\": 1\n}"));
        assert_eq!(as_json("not {json}"), None);
        assert_eq!(lang_for_path("src/main.rs"), Some("rust"));
        assert_eq!(lang_for_path("Dockerfile"), Some("dockerfile"));
        assert_eq!(lang_for_path("noext"), None);
    }
}
