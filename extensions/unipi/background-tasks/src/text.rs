//! Pure text helpers ported from unipi `types.ts` (names, durations, `/unipi:bg` arguments,
//! completion-delivery guidance, snapshot lists, the notification XML).

use crate::snapshot::{Snapshot, Status};

/// unipi `COMMAND_PREVIEW_CHARS`.
pub const COMMAND_PREVIEW_CHARS: usize = 90;
/// unipi `DEFAULT_MAX_BYTES` (pi tool-output cap).
pub const DEFAULT_MAX_BYTES: u32 = 30 * 1024;
/// unipi `DEFAULT_LOG_BYTES` (`min(DEFAULT_MAX_BYTES, 50 KiB)`).
pub const DEFAULT_LOG_BYTES: u32 = DEFAULT_MAX_BYTES;
/// unipi `MAX_LOG_BYTES`.
pub const MAX_LOG_BYTES: u32 = DEFAULT_MAX_BYTES;

/// Number of chars (unipi counts UTF-16 units; chars are the closest Rust analogue).
fn len(s: &str) -> usize {
    s.chars().count()
}

/// unipi `truncateChars`: cut to `max - 1` chars plus `…`.
#[must_use]
pub fn truncate_chars(value: &str, max: usize) -> String {
    if len(value) <= max {
        return value.to_owned();
    }
    let mut out: String = value.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// unipi `stripMatchingQuotes`.
#[must_use]
pub fn strip_matching_quotes(value: &str) -> &str {
    let t = value.trim();
    for q in ['"', '\''] {
        if t.len() >= 2 && t.starts_with(q) && t.ends_with(q) {
            return &t[1..t.len() - 1];
        }
    }
    t
}

/// unipi `compactWhitespace`.
#[must_use]
pub fn compact_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// unipi `normalizeTaskName` (≤ 80 chars, `None` when blank).
#[must_use]
pub fn normalize_task_name(value: Option<&str>) -> Option<String> {
    let normalized = compact_whitespace(strip_matching_quotes(value?));
    (!normalized.is_empty()).then(|| truncate_chars(&normalized, 80))
}

fn is_script_stop(c: char) -> bool {
    c.is_whitespace() || matches!(c, ';' | '&' | '|')
}

/// The `([^\s;&|]+)` token at the start of `s`.
fn script_token(s: &str) -> Option<&str> {
    let end = s.find(is_script_stop).unwrap_or(s.len());
    (end > 0).then(|| &s[..end])
}

/// unipi `deriveTaskNameFromCommand`.
#[must_use]
pub fn derive_task_name_from_command(command: &str) -> String {
    let normalized = compact_whitespace(strip_matching_quotes(command));
    if normalized.is_empty() {
        return "Background task".to_owned();
    }
    // /^(npm|pnpm|yarn|bun)\s+(?:(run)\s+)?([^\s;&|]+)/
    if let Some((runner, rest)) = normalized.split_once(' ')
        && matches!(runner, "npm" | "pnpm" | "yarn" | "bun")
    {
        let with_run = rest
            .strip_prefix("run ")
            .and_then(script_token)
            .map(|script| format!("{runner} run {script}"));
        if let Some(name) = with_run.or_else(|| script_token(rest).map(|s| format!("{runner} {s}")))
        {
            return truncate_chars(&name, 48);
        }
    }
    let words = normalized.split(' ').take(5).collect::<Vec<_>>().join(" ");
    truncate_chars(
        if words.is_empty() {
            &normalized
        } else {
            &words
        },
        48,
    )
}

/// unipi `taskDisplayName`: name → description → derived from command → id.
#[must_use]
pub fn task_display_name(
    name: Option<&str>,
    description: Option<&str>,
    command: Option<&str>,
    id: Option<&str>,
) -> String {
    normalize_task_name(name)
        .or_else(|| normalize_task_name(description))
        .or_else(|| {
            command
                .filter(|c| !c.is_empty())
                .map(derive_task_name_from_command)
        })
        .or_else(|| id.map(ToOwned::to_owned))
        .unwrap_or_else(|| "Background task".to_owned())
}

/// Parsed `/unipi:bg` arguments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BgArgs {
    /// `--name`.
    pub name: Option<String>,
    /// Command.
    pub command: String,
    /// `--agent`.
    pub is_agent: bool,
}

fn parse_name_value_and_rest(value_and_rest: &str) -> Option<(String, &str)> {
    let input = value_and_rest.trim_start();
    let first = input.chars().next()?;
    if first == '"' || first == '\'' {
        let mut escaped = false;
        let mut value = String::new();
        for (i, c) in input.char_indices().skip(1) {
            if escaped {
                value.push(c);
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == first {
                return Some((value, input[i + c.len_utf8()..].trim_start()));
            } else {
                value.push(c);
            }
        }
        return None;
    }
    let end = input.find(char::is_whitespace).unwrap_or(input.len());
    Some((input[..end].to_owned(), input[end..].trim_start()))
}

fn flag_rest<'a>(input: &'a str, flag: &str) -> Option<&'a str> {
    let rest = input.strip_prefix(flag)?;
    (rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')).then_some(rest)
}

/// unipi `parseBgCommandArgs`.
///
/// # Errors
///
/// `--name` without a (terminated) value.
pub fn parse_bg_command_args(args: &str) -> Result<BgArgs, String> {
    let mut input = args.trim();
    let mut out = BgArgs::default();
    'outer: while !input.is_empty() {
        for prefix in ["--name=", "-n="] {
            if let Some(rest) = input.strip_prefix(prefix) {
                let (value, rest) = parse_name_value_and_rest(rest).ok_or_else(|| {
                    format!("{} requires a task name", &prefix[..prefix.len() - 1])
                })?;
                out.name = normalize_task_name(Some(&value));
                input = rest;
                continue 'outer;
            }
        }
        for prefix in ["--name", "-n"] {
            if let Some(rest) = flag_rest(input, prefix) {
                let (value, rest) = parse_name_value_and_rest(rest)
                    .ok_or_else(|| format!("{prefix} requires a task name"))?;
                out.name = normalize_task_name(Some(&value));
                input = rest;
                continue 'outer;
            }
        }
        for (flag, agent) in [
            ("--agent", true),
            ("--llm-agent", true),
            ("--script", false),
            ("--no-agent", false),
        ] {
            if let Some(rest) = flag_rest(input, flag) {
                out.is_agent = agent;
                input = rest.trim_start();
                continue 'outer;
            }
        }
        if input == "--" {
            input = "";
        } else if let Some(rest) = input.strip_prefix("-- ") {
            input = rest.trim_start();
        }
        break;
    }
    input.clone_into(&mut out.command);
    Ok(out)
}

/// unipi `formatDuration`.
#[must_use]
pub fn format_duration(ms: i64) -> String {
    if ms < 1000 {
        return format!("{ms}ms");
    }
    let seconds = ms / 1000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let (minutes, rem_s) = (seconds / 60, seconds % 60);
    if minutes < 60 {
        return if rem_s > 0 {
            format!("{minutes}m{rem_s}s")
        } else {
            format!("{minutes}m")
        };
    }
    let (hours, rem_m) = (minutes / 60, minutes % 60);
    if rem_m > 0 {
        format!("{hours}h{rem_m}m")
    } else {
        format!("{hours}h")
    }
}

/// JS `Number.prototype.toFixed(1)` for non-negative display values (ties round up).
#[must_use]
pub fn to_fixed1(x: f64) -> String {
    let scaled = (x * 10.0).round() / 10.0;
    format!("{scaled:.1}")
}

/// JS `Math.round` for non-negative display values.
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "non-negative display values"
)]
fn js_round(x: f64) -> u64 {
    (x + 0.5).floor() as u64
}

/// unipi `formatCompactNumber`.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "display only")]
pub fn format_compact_number(count: u64) -> String {
    let f = count as f64;
    match count {
        0..=999 => count.to_string(),
        1_000..=9_999 => format!("{}k", to_fixed1(f / 1_000.0)),
        10_000..=999_999 => format!("{}k", js_round(f / 1_000.0)),
        1_000_000..=9_999_999 => format!("{}M", to_fixed1(f / 1_000_000.0)),
        _ => format!("{}M", js_round(f / 1_000_000.0)),
    }
}

/// pi `formatSize`: `B`, `x.xKB`, `x.xMB`.
#[must_use]
#[allow(clippy::cast_precision_loss, reason = "display only")]
pub fn format_size(bytes: u64) -> String {
    let f = bytes as f64;
    if bytes < 1024 {
        format!("{bytes}B")
    } else if bytes < 1024 * 1024 {
        format!("{}KB", to_fixed1(f / 1024.0))
    } else {
        format!("{}MB", to_fixed1(f / (1024.0 * 1024.0)))
    }
}

/// unipi `escapeXml`.
#[must_use]
pub fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// unipi `normalizeMaxBytes` (`1..=MAX_LOG_BYTES`).
#[must_use]
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "clamped to 1..=MAX_LOG_BYTES first"
)]
pub fn normalize_max_bytes(value: Option<f64>, fallback: u32) -> u32 {
    let raw = value
        .filter(|v| v.is_finite())
        .map_or(f64::from(fallback), f64::floor);
    raw.clamp(1.0, f64::from(MAX_LOG_BYTES)) as u32
}

/// unipi completion-delivery mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryMode {
    /// Notification + automatic follow-up turn.
    NotificationAndWake,
    /// Notification without a turn.
    NotificationOnly,
    /// Nothing is delivered.
    ManualMonitoring,
}

/// unipi `deriveCompletionDeliveryGuidance` → `(mode, text)`.
#[must_use]
pub fn completion_delivery_guidance(notify: bool, trigger: bool) -> (DeliveryMode, String) {
    if notify && trigger {
        return (
            DeliveryMode::NotificationAndWake,
            [
                "Terminal notification: enabled.",
                "Automatic follow-up turn: enabled.",
                "Next action: do not poll or sleep merely to wait; continue only independent useful work, otherwise end this turn and wait for <background-task-notification>.",
            ]
            .join("\n"),
        );
    }
    if notify {
        return (
            DeliveryMode::NotificationOnly,
            [
                "Terminal notification: enabled.",
                "Automatic follow-up turn: disabled. The terminal notification will be delivered, but it will not start an agent turn.",
                "Next action: automatic wake-up was explicitly disabled; use bg_status/bg_logs only when deliberate monitoring is required, without tight polling.",
            ]
            .join("\n"),
        );
    }
    let wake = if trigger {
        "Automatic follow-up turn: disabled because terminal notifications are disabled. triggerOnCompletion has no effect while notifyOnCompletion is false."
    } else {
        "Automatic follow-up turn: disabled."
    };
    (
        DeliveryMode::ManualMonitoring,
        [
            "Terminal notification: disabled.",
            wake,
            "Next action: completion delivery was explicitly disabled; use bg_status/bg_logs only for deliberate manual monitoring, without tight polling.",
        ]
        .join("\n"),
    )
}

/// Status glyph of the snapshot list.
fn status_icon(status: Status) -> &'static str {
    match status {
        Status::Running => "\u{25b6}",
        Status::Completed => "\u{2713}",
        Status::Killed => "\u{25a0}",
        Status::Failed => "\u{2717}",
    }
}

/// unipi `formatSnapshotList` (Oino: "this Oino extension runtime").
#[must_use]
pub fn format_snapshot_list(tasks: &[Snapshot], now: i64) -> String {
    if tasks.is_empty() {
        return "No background tasks in this Oino extension runtime.".to_owned();
    }
    tasks
        .iter()
        .map(|t| {
            let age = format_duration(t.end_time.unwrap_or(now) - t.start_time);
            let code = match t.exit_code {
                Some(Some(c)) => format!(" exit={c}"),
                Some(None) => " exit=null".to_owned(),
                None => String::new(),
            };
            let pid = t.pid.map(|p| format!(" pid={p}")).unwrap_or_default();
            let error = t
                .error
                .as_deref()
                .filter(|e| !e.is_empty())
                .map(|e| format!(" error={}", truncate_chars(e, 80)))
                .unwrap_or_default();
            format!(
                "{} {} {} {age}{code}{pid} \u{2014} {}{error}\n    output: {}",
                status_icon(t.status),
                t.id,
                t.status.as_str(),
                truncate_chars(&t.display_name(), COMMAND_PREVIEW_CHARS),
                t.output_path
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The `<background-task-notification>` custom-message text (unipi `notifyCompletion`).
#[must_use]
pub fn notification_text(t: &Snapshot) -> String {
    let name = t.display_name();
    let guidance = "Terminal state and output metadata are durable. Do not call bg_status to reconfirm; use bg_logs only if output is needed.";
    let mut lines = vec![
        "<background-task-notification>".to_owned(),
        format!("  <task-id>{}</task-id>", t.id),
        format!("  <task-name>{}</task-name>", escape_xml(&name)),
        format!("  <status>{}</status>", t.status.as_str()),
    ];
    match t.exit_code {
        Some(Some(c)) => lines.push(format!("  <exit-code>{c}</exit-code>")),
        Some(None) => lines.push("  <exit-code>null</exit-code>".to_owned()),
        None => {}
    }
    if let Some(e) = t.error.as_deref().filter(|e| !e.is_empty()) {
        lines.push(format!("  <error>{}</error>", escape_xml(e)));
    }
    let quoted = serde_json::to_string(&name).unwrap_or_default();
    lines.push(format!(
        "  <output-file>{}</output-file>",
        escape_xml(&t.output_path)
    ));
    lines.push(format!(
        "  <summary>{}</summary>",
        escape_xml(&format!("Background task {quoted} {}", t.status.as_str()))
    ));
    lines.push(format!("  <guidance>{}</guidance>", escape_xml(guidance)));
    lines.push("</background-task-notification>".to_owned());
    lines.join("\n")
}

/// The `bg_logs` text (unipi `getTaskLogs`): `content` is the slice read, `total` the file size.
#[must_use]
pub fn logs_text(content: &str, total: u64, tail: bool, output_path: &str) -> (String, bool) {
    let read = content.len() as u64;
    let truncated = total > read;
    let body = if content.is_empty() {
        "(no output yet)".to_owned()
    } else {
        content.to_owned()
    };
    if truncated {
        let direction = if tail { "tail" } else { "head" };
        let notice = format!(
            "\n\n[Showing {direction} {} of {}; {} omitted. Full output: {output_path}]",
            format_size(read),
            format_size(total),
            format_size(total - read)
        );
        let text = if tail {
            format!("{notice}\n\n{body}")
        } else {
            format!("{body}{notice}")
        };
        (text, true)
    } else {
        (format!("{body}\n\n[Full output: {output_path}]"), false)
    }
}

/// unipi `pendingWakeReason`: `bg: <first> [+N more]` over running tasks that wake the agent.
#[must_use]
pub fn pending_wake_reason(tasks: &[Snapshot]) -> Option<String> {
    let wake: Vec<&Snapshot> = tasks
        .iter()
        .filter(|t| t.status == Status::Running && t.trigger_on_completion)
        .collect();
    let first = wake.first()?;
    let more = if wake.len() > 1 {
        format!(" +{} more", wake.len() - 1)
    } else {
        String::new()
    };
    Some(format!("bg: {}{more}", first.display_name()))
}

/// Last [`crate::snapshot::OUTPUT_TAIL_LINES`] non-empty output lines (ANSI stripped, ≤ 200
/// chars each), unipi `appendOutputTail`.
#[must_use]
pub fn output_tail(text: &str, lines: usize) -> Vec<String> {
    let mut out: Vec<String> = text
        .split('\n')
        .map(|l| strip_ansi(l.trim_end_matches('\r')).trim_end().to_owned())
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            if len(&l) > 200 {
                format!("{}…", l.chars().take(200).collect::<String>())
            } else {
                l
            }
        })
        .collect();
    let skip = out.len().saturating_sub(lines);
    out.drain(..skip);
    out
}

/// Removes `ESC [ … letter` sequences.
#[must_use]
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            while let Some(&n) = chars.peek() {
                chars.next();
                if n.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// unipi `trayElapsed`.
#[must_use]
pub fn tray_elapsed(ms: i64) -> String {
    let s = (ms / 1000).max(0);
    if s < 60 {
        return format!("{s}s");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m}m{:02}s", s % 60);
    }
    format!("{}h{:02}m", m / 60, m % 60)
}

/// unipi `trayPlural`.
#[must_use]
pub fn tray_plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bg_names_aliases_quotes_and_empty_commands() {
        let p = |s: &str| parse_bg_command_args(s).unwrap_or_default();
        assert_eq!(
            p("--name \"Build Docs\" npm run docs"),
            BgArgs {
                name: Some("Build Docs".into()),
                command: "npm run docs".into(),
                is_agent: false
            }
        );
        assert_eq!(p("--name=Build npm test").name.as_deref(), Some("Build"));
        assert_eq!(p("--name=Build npm test").command, "npm test");
        assert_eq!(
            p("-n 'Quoted Name' printf ok").name.as_deref(),
            Some("Quoted Name")
        );
        assert_eq!(p("-n=One printf one").command, "printf one");
        let a = p("--agent --name Agent pi -p hi");
        assert!(a.is_agent);
        assert_eq!(
            (a.name.as_deref(), a.command.as_str()),
            (Some("Agent"), "pi -p hi")
        );
        assert!(!p("--name Agent --script pi -p hi").is_agent);
        assert_eq!(
            p("echo ok"),
            BgArgs {
                name: None,
                command: "echo ok".into(),
                is_agent: false
            }
        );
        let only = p("--name 'Only Name'");
        assert_eq!(
            (only.name.as_deref(), only.command.as_str()),
            (Some("Only Name"), "")
        );
        assert_eq!(p("-- --name x").command, "--name x");
        assert!(
            parse_bg_command_args("--name")
                .unwrap_err()
                .contains("requires a task name")
        );
        assert!(parse_bg_command_args("--name \"unterminated").is_err());
    }

    #[test]
    fn normalizes_names_and_derives_fallbacks() {
        assert_eq!(
            normalize_task_name(Some(" \"A   B\" ")).as_deref(),
            Some("A B")
        );
        assert_eq!(normalize_task_name(Some("\n\t")), None);
        assert_eq!(normalize_task_name(None), None);
        assert_eq!(
            normalize_task_name(Some(&"x".repeat(100))),
            Some(format!("{}…", "x".repeat(79)))
        );
        assert_eq!(
            derive_task_name_from_command("npm run test -- --watch"),
            "npm run test"
        );
        assert_eq!(
            derive_task_name_from_command("pnpm build && echo done"),
            "pnpm build"
        );
        assert_eq!(derive_task_name_from_command(""), "Background task");
        assert_eq!(
            derive_task_name_from_command("python -m http.server 8080 --bind 0.0.0.0"),
            "python -m http.server 8080 --bind"
        );
        assert_eq!(task_display_name(None, None, None, Some("b123")), "b123");
        assert_eq!(
            task_display_name(None, Some("desc"), Some("ls"), None),
            "desc"
        );
        assert_eq!(
            task_display_name(None, None, Some("cargo test -p x"), None),
            "cargo test -p x"
        );
    }

    #[test]
    fn completion_delivery_for_every_combination() {
        let (m, t) = completion_delivery_guidance(true, true);
        assert_eq!(m, DeliveryMode::NotificationAndWake);
        assert!(
            t.starts_with("Terminal notification: enabled.\nAutomatic follow-up turn: enabled.")
        );
        assert_eq!(
            completion_delivery_guidance(true, false).0,
            DeliveryMode::NotificationOnly
        );
        let (m, t) = completion_delivery_guidance(false, true);
        assert_eq!(m, DeliveryMode::ManualMonitoring);
        assert!(t.contains("triggerOnCompletion has no effect"));
        let (_, t) = completion_delivery_guidance(false, false);
        assert!(t.contains("deliberate manual monitoring"));
        assert!(!t.contains("triggerOnCompletion has no effect"));
    }

    #[test]
    fn formats_durations_numbers_and_limits() {
        assert_eq!(format_duration(999), "999ms");
        assert_eq!(format_duration(1000), "1s");
        assert_eq!(format_duration(65_000), "1m5s");
        assert_eq!(format_duration(120_000), "2m");
        assert_eq!(format_duration(3_660_000), "1h1m");
        assert_eq!(format_compact_number(999), "999");
        assert_eq!(format_compact_number(1250), "1.3k");
        assert_eq!(format_compact_number(42_000), "42k");
        assert_eq!(format_compact_number(2_500_000), "2.5M");
        assert_eq!(format_size(1023), "1023B");
        assert_eq!(format_size(1536), "1.5KB");
        assert_eq!(format_size(5 * 1024 * 1024), "5.0MB");
        assert_eq!(normalize_max_bytes(Some(-1.0), 123), 1);
        assert_eq!(normalize_max_bytes(Some(f64::NAN), 123), 123);
        assert_eq!(normalize_max_bytes(Some(1.9), 123), 1);
        assert_eq!(normalize_max_bytes(Some(1e9), 123), MAX_LOG_BYTES);
        assert_eq!(normalize_max_bytes(None, DEFAULT_LOG_BYTES), 30 * 1024);
        assert_eq!(truncate_chars("abcdef", 4), "abc…");
        assert_eq!(tray_elapsed(70_000), "1m10s");
        assert_eq!(tray_elapsed(3_700_000), "1h01m");
        assert_eq!(tray_plural(1, "task"), "1 task");
        assert_eq!(tray_plural(2, "task"), "2 tasks");
    }

    #[test]
    fn logs_text_marks_truncation() {
        let (t, trunc) = logs_text("abc", 3, true, "/o.log");
        assert!(!trunc);
        assert_eq!(t, "abc\n\n[Full output: /o.log]");
        let (t, trunc) = logs_text("", 0, true, "/o.log");
        assert!(!trunc && t.starts_with("(no output yet)"));
        let (t, trunc) = logs_text("xyz", 2048, true, "/o.log");
        assert!(trunc);
        assert!(t.starts_with(
            "\n\n[Showing tail 3B of 2.0KB; 2.0KB omitted. Full output: /o.log]\n\nxyz"
        ));
        let (t, _) = logs_text("xyz", 2048, false, "/o.log");
        assert!(t.starts_with("xyz\n\n[Showing head"));
    }

    #[test]
    fn output_tail_keeps_last_non_empty_lines() {
        let text = "a\n\n\u{1b}[31mb\u{1b}[0m\nc  \nd\n";
        assert_eq!(output_tail(text, 3), ["b", "c", "d"]);
        assert_eq!(output_tail(&"y".repeat(300), 3)[0].chars().count(), 201);
    }
}
