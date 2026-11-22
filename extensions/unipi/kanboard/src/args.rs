//! Command texts and argument parsing (unipi `commands.ts`: help texts, `parseAddArgs`,
//! `detectFilePaths`, open flags, board URLs, completions).

use oino_extension::command::CompletionItem;

use crate::shapes::Task;
use crate::shell::tokenize_args;

/// `/kanboard` subcommands.
pub const SUBCOMMANDS: &[&str] = &["open", "close", "status", "doctor", "show"];
/// Display-only custom types (never LLM context).
pub const HELP_CUSTOM_TYPE: &str = "unipi:kanboard-help";
/// Doctor report.
pub const DOCTOR_CUSTOM_TYPE: &str = "unipi:kanboard-doctor";
/// In-chat board.
pub const SHOW_CUSTOM_TYPE: &str = "unipi:kanboard-show";

/// Bare `/kanboard`.
pub const HELP: &str = "Kanboard commands
  /kanboard                       this list
  /kanboard open [--host 127.0.0.1|0.0.0.0|tailscale] [--port N]
                                        start the board and open the board panel
  /kanboard close                 shut the board down
  /kanboard status                board and claims
  /kanboard show [<lane>] [--all] the board, in chat (lanes + claim order)
  /kanboard doctor                check the setup
  /kanboard-add [-p 1-5] <title>  add a task; lines below the title are the description (paste images there)
  Superagent                    manage boards and dispatch workers across projects
Priority -p: 1 none · 2 low · 3 medium · 4 high · 5 urgent";

/// Board contract for compaction contexts (unipi `kanboardClaimsReminder`).
#[must_use]
pub fn claims_reminder(claims: &[String], cli: Option<&str>) -> Option<String> {
    if claims.is_empty() {
        return None;
    }
    let cli = cli.unwrap_or(crate::shell::BINARY);
    let ids = claims.join(", ");
    Some(format!(
        "Kanboard: this session holds {ids} In Progress. Never leave a claimed task In Progress — when its work is done run `{cli} finish <ID> --comment \"<summary>\"`, or `{cli} move <ID> blocked --comment \"<what you need>\"` if you are stuck. Read a task with `{cli} show <ID>`."
    ))
}

/// Footer glance label (unipi `kanboardGlanceLabel`).
#[must_use]
pub fn glance_label(claims: &[String], autowork: bool) -> Option<String> {
    let claim = match claims.len() {
        0 => None,
        1 => Some(claims[0].clone()),
        n => Some(format!("{} +{}", claims[0], n - 1)),
    };
    let parts: Vec<String> = claim
        .into_iter()
        .chain(autowork.then(|| "autowork".to_owned()))
        .collect();
    (!parts.is_empty()).then(|| format!("▣ {}", parts.join(" · ")))
}

/// Parsed `/kanboard-add`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AddArgs {
    /// Title (may be empty).
    pub title: String,
    /// CLI argv after the title (`--after`, `--priority`, `--status`).
    pub flags: Vec<String>,
    /// Lines below the first — the task body.
    pub description: String,
    /// Path-like tokens in the description (passed as `--attach-existing`).
    pub attaches: Vec<String>,
    /// Usage error.
    pub error: Option<String>,
}

fn priority_name(n: &str) -> Option<&'static str> {
    Some(match n {
        "1" => "none",
        "2" => "low",
        "3" => "medium",
        "4" => "high",
        "5" => "urgent",
        _ => return None,
    })
}

fn is_dash_p_digit(word: &str) -> bool {
    word.len() == 3 && word.starts_with("-p") && word.as_bytes()[2].is_ascii_digit()
}

/// Parses `[-p 1-5 | --priority P] [--after ID]… [--status backlog|todo] <title>`: flags come
/// first on the first line, everything after them is the title, every later line the body.
#[must_use]
pub fn parse_add_args(text: &str) -> AddArgs {
    let mut lines = text.split('\n');
    let words = tokenize_args(lines.next().unwrap_or(""));
    let rest: Vec<&str> = lines.collect();
    let mut flags = Vec::new();
    let fail = |flags: Vec<String>, e: String| AddArgs {
        flags,
        error: Some(e),
        ..AddArgs::default()
    };
    let mut index = 0;
    while index < words.len() {
        let word = words[index].as_str();
        let (name, inline) = match word.find('=') {
            Some(eq) if eq > 0 => (&word[..eq], Some(&word[eq + 1..])),
            _ => (word, None),
        };
        let take = |index: &mut usize| -> Option<String> {
            if let Some(v) = inline {
                return Some(v.to_owned());
            }
            let v = words.get(*index + 1).cloned();
            if v.is_some() {
                *index += 1;
            }
            v
        };
        if name == "-p" || is_dash_p_digit(word) {
            let raw = if is_dash_p_digit(word) {
                Some(word[2..].to_owned())
            } else {
                take(&mut index)
            };
            let Some(priority) = raw.as_deref().and_then(priority_name) else {
                return fail(flags, "-p takes 1-5 (1 none · 5 urgent)".into());
            };
            flags.extend(["--priority".to_owned(), priority.to_owned()]);
            index += 1;
            continue;
        }
        if matches!(name, "--priority" | "--status" | "--after") {
            match take(&mut index) {
                Some(v) if !v.is_empty() => flags.extend([name.to_owned(), v]),
                _ => return fail(flags, format!("{name} needs a value")),
            }
            index += 1;
            continue;
        }
        break;
    }
    let title = words[index.min(words.len())..].join(" ").trim().to_owned();
    let description = rest.join("\n").trim().to_owned();
    let attaches = detect_file_paths(&description);
    AddArgs {
        title,
        flags,
        description,
        attaches,
        error: None,
    }
}

/// Whitespace-free tokens that are absolute (`/…`) or `~/` paths — pasted screenshots and
/// logs. unipi checked existence here; the guest cannot see host files, so the binary
/// keeps only the existing ones (`add --attach-existing`).
#[must_use]
pub fn detect_file_paths(text: &str) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for raw in text.split_whitespace() {
        let mut token = raw;
        if let Some(inner) = unwrap_link(token) {
            token = inner;
        }
        let token = token.trim_end_matches(['.', ',', ';', ':']);
        if !(token.starts_with('/') || token.starts_with("~/")) || token.len() < 2 {
            continue;
        }
        if !found.iter().any(|f| f == token) {
            found.push(token.to_owned());
        }
    }
    found
}

/// `![alt](path)` / `[x](path)` / `(path)` with an optional trailing punctuation mark.
fn unwrap_link(token: &str) -> Option<&str> {
    let t = token.strip_suffix(['.', ',', ';', ':']).unwrap_or(token);
    let t = t.strip_suffix(')')?;
    if let Some(rest) = t.strip_prefix('(') {
        return (!rest.is_empty()).then_some(rest);
    }
    let rest = t.strip_prefix('!').unwrap_or(t).strip_prefix('[')?;
    let close = rest.find("](")?;
    if rest[..close].contains(']') {
        return None;
    }
    let inner = &rest[close + 2..];
    (!inner.is_empty()).then_some(inner)
}

/// `open` flags overriding the settings for this call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenArgs {
    /// Bind host.
    pub host: String,
    /// Port.
    pub port: u16,
    /// Unknown words (reported).
    pub unknown: Vec<String>,
}

/// Splits `open`'s arguments.
#[must_use]
pub fn parse_open_args(args: &str, host: &str, port: u16) -> OpenArgs {
    let tokens: Vec<&str> = args.split_whitespace().collect();
    let mut out = OpenArgs {
        host: host.to_owned(),
        port,
        unknown: Vec::new(),
    };
    let mut i = 0;
    while i < tokens.len() {
        let flag = tokens[i];
        let mut take_value = |inline: Option<&str>| -> Option<String> {
            inline.map(ToOwned::to_owned).or_else(|| {
                i += 1;
                tokens.get(i).map(|s| (*s).to_owned())
            })
        };
        if flag == "--host" || flag.starts_with("--host=") {
            match take_value(flag.strip_prefix("--host=")) {
                Some(h) if !h.is_empty() => out.host = h,
                _ => out.unknown.push(flag.to_owned()),
            }
        } else if flag == "--port" || flag.starts_with("--port=") {
            let given = take_value(flag.strip_prefix("--port="));
            match given.as_deref().and_then(|g| g.parse::<u16>().ok()) {
                Some(p) => out.port = p,
                None => out.unknown.push(
                    format!("{flag} {}", given.unwrap_or_default())
                        .trim()
                        .to_owned(),
                ),
            }
        } else {
            out.unknown.push(flag.to_owned());
        }
        i += 1;
    }
    out
}

/// Loopback host (`localhost`, `::1`, `127.*`).
#[must_use]
pub fn is_loopback_host(host: &str) -> bool {
    let bare = host.trim().trim_start_matches('[').trim_end_matches(']');
    bare == "localhost" || bare == "::1" || bare.starts_with("127.") || bare == "0:0:0:0:0:0:0:1"
}

/// The URLs to hand the user, plus warnings.
#[must_use]
pub fn format_board_urls(
    host: &str,
    port: u16,
    slug: Option<&str>,
    token: Option<&str>,
    hostname: Option<&str>,
    tailscale: Option<&str>,
) -> (Vec<String>, Vec<String>) {
    let path = slug.map_or_else(|| "/".to_owned(), |s| format!("/p/{s}"));
    let suffix = token.map(|t| format!("?t={t}")).unwrap_or_default();
    if is_loopback_host(host) {
        return (
            vec![format!("http://127.0.0.1:{port}{path}{suffix}")],
            vec![format!(
                "from another machine, tunnel it: ssh -N -L {port}:127.0.0.1:{port} {}",
                hostname.unwrap_or("$(hostname)")
            )],
        );
    }
    let mut hosts: Vec<String> = Vec::new();
    if host == "0.0.0.0" || host == "::" {
        hosts.extend(hostname.map(ToOwned::to_owned));
        hosts.extend(tailscale.map(ToOwned::to_owned));
    } else {
        hosts.push(host.to_owned());
    }
    if hosts.is_empty() {
        hosts.push(host.to_owned());
    }
    (
        hosts
            .iter()
            .map(|h| format!("http://{h}:{port}{path}{suffix}"))
            .collect(),
        vec!["board is reachable from the network; anyone with the link can edit it".to_owned()],
    )
}

/// First non-empty line of `tailscale ip -4`.
#[must_use]
pub fn first_line(output: &str) -> Option<String> {
    output
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(ToOwned::to_owned)
}

fn item(value: String, label: &str, description: &str) -> CompletionItem {
    CompletionItem {
        value,
        label: Some(label.to_owned()),
        description: Some(description.to_owned()),
    }
}

/// Every lane `show` can filter.
pub const SHOW_FILTERS: &[&str] = &[
    "backlog",
    "todo",
    "in_progress",
    "blocked",
    "in_review",
    "done",
    "cancelled",
    "archived",
];

fn sub_description(sub: &str) -> &'static str {
    match sub {
        "open" => "Start the daemon and open the board panel (--host 0.0.0.0|tailscale, --port N)",
        "close" => "Shut down the board daemon",
        "status" => "Daemon, project counts and claims",
        "doctor" => "Check the whole setup (binary, daemon, project, agent, claims)",
        _ => "The board in chat: show <lane> filters a lane; --all adds cancelled/archived",
    }
}

/// `/kanboard …` completions; every value is the full argument text.
#[must_use]
pub fn kanboard_completions(prefix: &str) -> Vec<CompletionItem> {
    let raw = prefix.trim_start();
    let (before, last) = match raw.rfind(char::is_whitespace) {
        Some(i) => (&raw[..=i], &raw[i + 1..]),
        None => ("", raw),
    };
    let words: Vec<&str> = before.split_whitespace().collect();
    if words == ["open", "--host"] {
        return [
            ("127.0.0.1", "local only (default)"),
            ("0.0.0.0", "every interface, token-gated"),
            ("tailscale", "the tailnet IPv4"),
        ]
        .into_iter()
        .filter(|(v, _)| v.starts_with(last))
        .map(|(v, d)| item(format!("{before}{v}"), v, d))
        .collect();
    }
    if words.first() == Some(&"open") && last.starts_with("--") {
        return [
            ("--host", "Bind address: 127.0.0.1 · 0.0.0.0 · tailscale"),
            ("--port", "Port to bind (0 = auto)"),
        ]
        .into_iter()
        .filter(|(v, _)| v.starts_with(last))
        .map(|(v, d)| item(format!("{before}{v}"), v, d))
        .collect();
    }
    if words == ["show"] {
        let mut out: Vec<CompletionItem> = SHOW_FILTERS
            .iter()
            .filter(|l| l.starts_with(last))
            .map(|l| item(format!("{before}{l}"), l, &format!("Show only {l}")))
            .collect();
        if "--all".starts_with(last) {
            out.push(item(
                format!("{before}--all"),
                "--all",
                "Include cancelled and archived lanes",
            ));
        }
        return out;
    }
    if !words.is_empty() {
        return Vec::new();
    }
    let needle = last.to_lowercase();
    SUBCOMMANDS
        .iter()
        .filter(|s| s.starts_with(&needle))
        .map(|s| item((*s).to_owned(), s, sub_description(s)))
        .collect()
}

/// Task-id completions over a cached board (`id-title` rows when `id_title`).
#[must_use]
pub fn task_completions(
    tasks: &[Task],
    prefix: &str,
    before: &str,
    title_search: bool,
    id_title: bool,
) -> Vec<CompletionItem> {
    let needle = prefix.to_lowercase();
    tasks
        .iter()
        .filter(|t| t.status != "cancelled" && t.status != "archived")
        .filter(|t| {
            t.id.to_lowercase().starts_with(&needle)
                || (title_search && t.shown_title().to_lowercase().contains(&needle))
        })
        .take(12)
        .map(|t| {
            if id_title {
                let title: String = t.shown_title().chars().take(64).collect();
                item(
                    format!("{before}{}", t.id),
                    &format!("{}  {title}", t.id),
                    &t.status,
                )
            } else {
                item(
                    format!("{before}{}", t.id),
                    &t.id,
                    &format!("{} — {}", t.status, t.shown_title()),
                )
            }
        })
        .collect()
}

/// What `/kanboard-add` completes: either items, or a request for task ids.
#[derive(Clone, Debug)]
pub enum AddCompletion {
    /// Static items.
    Items(Vec<CompletionItem>),
    /// Task ids after `--after`: `(prefix, before)`.
    Tasks(String, String),
}

const PRIORITIES: &[(&str, &str)] = &[
    ("1", "none"),
    ("2", "low"),
    ("3", "medium"),
    ("4", "high"),
    ("5", "urgent"),
];

/// `/kanboard-add …` completions.
#[must_use]
pub fn add_completions(prefix: &str) -> AddCompletion {
    let (before, token) = match prefix.rfind(char::is_whitespace) {
        Some(i) => (&prefix[..=i], &prefix[i + 1..]),
        None => ("", prefix),
    };
    let prev = before.split_whitespace().last();
    if prev == Some("--after") {
        return AddCompletion::Tasks(token.to_owned(), before.to_owned());
    }
    if token == "--after" {
        return AddCompletion::Tasks(String::new(), format!("{prefix} "));
    }
    if prev == Some("-p") || token == "-p" {
        let (pb, digits) = if token == "-p" {
            (format!("{prefix} "), "")
        } else {
            (before.to_owned(), token)
        };
        return AddCompletion::Items(
            PRIORITIES
                .iter()
                .filter(|(n, _)| n.starts_with(digits))
                .map(|(n, name)| item(format!("{pb}{n}"), &format!("-p {n}"), name))
                .collect(),
        );
    }
    let value_list = |flag: &str, values: &[&str], description: &str| -> Option<AddCompletion> {
        let (vb, part) = if prev == Some(flag) {
            (before.to_owned(), token)
        } else if token == flag {
            (format!("{prefix} "), "")
        } else {
            return None;
        };
        Some(AddCompletion::Items(
            values
                .iter()
                .filter(|v| v.starts_with(part))
                .map(|v| item(format!("{vb}{v}"), v, description))
                .collect(),
        ))
    };
    if let Some(out) = value_list(
        "--priority",
        &["none", "low", "medium", "high", "urgent"],
        "priority",
    ) {
        return out;
    }
    if let Some(out) = value_list("--status", &["backlog", "todo"], "lane") {
        return out;
    }
    if token.starts_with('-') {
        return AddCompletion::Items(
            ["-p", "--priority", "--status", "--after"]
                .into_iter()
                .filter(|f| f.starts_with(token))
                .map(|f| {
                    let d = match f {
                        "-p" | "--priority" => "Priority: none · low · medium · high · urgent",
                        "--status" => "Lane: backlog|todo",
                        _ => "Runs after <task id>",
                    };
                    item(format!("{before}{f}"), f, d)
                })
                .collect(),
        );
    }
    AddCompletion::Items(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(items: &[CompletionItem]) -> Vec<String> {
        items.iter().map(|i| i.value.clone()).collect()
    }

    #[test]
    fn parses_flags_before_the_title() {
        let a = parse_add_args("-p 4 --after KB-1 --status todo fix the \"login\" bug");
        assert_eq!(
            a.flags,
            ["--priority", "high", "--after", "KB-1", "--status", "todo"]
        );
        assert_eq!(a.title, "fix the login bug");
        let b = parse_add_args("-p5 --priority=low x");
        assert_eq!(b.flags, ["--priority", "urgent", "--priority", "low"]);
        assert_eq!(parse_add_args("-p 1 x").flags, ["--priority", "none"]);
    }

    #[test]
    fn usage_errors() {
        assert_eq!(
            parse_add_args("-p 9 x").error.as_deref(),
            Some("-p takes 1-5 (1 none · 5 urgent)")
        );
        assert_eq!(
            parse_add_args("--after").error.as_deref(),
            Some("--after needs a value")
        );
        assert_eq!(
            parse_add_args("--status=").error.as_deref(),
            Some("--status needs a value")
        );
        let empty = parse_add_args("");
        assert!(empty.title.is_empty() && empty.description.is_empty() && empty.error.is_none());
    }

    #[test]
    fn description_lines_and_paths() {
        let a =
            parse_add_args("title\nsee /tmp/shot.png, and ![x](~/logs/a.log).\nrelative/x.txt /");
        assert_eq!(a.title, "title");
        assert_eq!(
            a.description,
            "see /tmp/shot.png, and ![x](~/logs/a.log).\nrelative/x.txt /"
        );
        assert_eq!(a.attaches, ["/tmp/shot.png", "~/logs/a.log"]);
        assert_eq!(detect_file_paths("(/a/b) /a/b"), ["/a/b"]);
    }

    #[test]
    fn open_args_override_settings_for_this_call() {
        let o = parse_open_args("--host 0.0.0.0 --port=8080 junk", "127.0.0.1", 0);
        assert_eq!((o.host.as_str(), o.port), ("0.0.0.0", 8080));
        assert_eq!(o.unknown, ["junk"]);
        let bad = parse_open_args("--port 99999", "h", 5);
        assert_eq!(
            (bad.port, bad.unknown),
            (5, vec!["--port 99999".to_owned()])
        );
        assert_eq!(parse_open_args("", "h", 1).host, "h");
    }

    #[test]
    fn loopback_and_urls() {
        for h in ["localhost", "127.0.0.1", "127.1.2.3", "::1", "[::1]"] {
            assert!(is_loopback_host(h), "{h}");
        }
        assert!(!is_loopback_host("0.0.0.0"));
        let (urls, warn) = format_board_urls("127.0.0.1", 9, Some("p-1"), None, Some("box"), None);
        assert_eq!(urls, ["http://127.0.0.1:9/p/p-1"]);
        assert_eq!(
            warn,
            ["from another machine, tunnel it: ssh -N -L 9:127.0.0.1:9 box"]
        );
        let (urls, warn) = format_board_urls(
            "0.0.0.0",
            9,
            None,
            Some("T"),
            Some("box"),
            Some("100.1.1.1"),
        );
        assert_eq!(urls, ["http://box:9/?t=T", "http://100.1.1.1:9/?t=T"]);
        assert!(warn[0].contains("anyone with the link"));
        assert_eq!(
            format_board_urls("10.0.0.2", 9, None, None, None, None).0,
            ["http://10.0.0.2:9/"]
        );
        assert_eq!(first_line("\n 100.64.0.1 \nx"), Some("100.64.0.1".into()));
    }

    #[test]
    fn help_lists_kanboard_commands() {
        assert!(HELP.starts_with("Kanboard commands\n"));
    }

    #[test]
    fn reminder_and_glance_label() {
        assert_eq!(claims_reminder(&[], None), None);
        let r = claims_reminder(&["A-1".into()], Some("/b --actor agent --project p")).unwrap();
        assert!(r.starts_with("Kanboard: this session holds A-1 In Progress."));
        assert_eq!(glance_label(&[], false), None);
        assert_eq!(glance_label(&["A-1".into()], false).unwrap(), "▣ A-1");
        assert_eq!(
            glance_label(&["A-1".into(), "A-2".into()], true).unwrap(),
            "▣ A-1 +1 · autowork"
        );
        assert_eq!(glance_label(&[], true).unwrap(), "▣ autowork");
    }

    #[test]
    fn kanboard_completions_keep_the_full_prefix() {
        assert_eq!(values(&kanboard_completions("o")), ["open"]);
        assert_eq!(values(&kanboard_completions("cl")), ["close"]);
        assert_eq!(
            values(&kanboard_completions("open --host 0")),
            ["open --host 0.0.0.0"]
        );
        assert_eq!(
            values(&kanboard_completions("open --")),
            ["open --host", "open --port"]
        );
        let show = values(&kanboard_completions("show "));
        assert_eq!(show.len(), 9);
        assert_eq!(show[8], "show --all");
        assert_eq!(
            values(&kanboard_completions("show in")),
            ["show in_progress", "show in_review"]
        );
    }

    #[test]
    fn add_completion_shapes() {
        let AddCompletion::Items(p) = add_completions("-p ") else {
            panic!()
        };
        assert_eq!(values(&p), ["-p 1", "-p 2", "-p 3", "-p 4", "-p 5"]);
        let AddCompletion::Items(s) = add_completions("x --status b") else {
            panic!()
        };
        assert_eq!(values(&s), ["x --status backlog"]);
        let AddCompletion::Items(f) = add_completions("--p") else {
            panic!()
        };
        assert_eq!(values(&f), ["--priority"]);
        assert!(
            matches!(add_completions("--after K"), AddCompletion::Tasks(p, b) if p == "K" && b == "--after ")
        );
        let tasks = vec![
            Task {
                id: "KB-1".into(),
                title: "login".into(),
                status: "todo".into(),
                ..Task::default()
            },
            Task {
                id: "KB-2".into(),
                title: "kbd".into(),
                status: "cancelled".into(),
                ..Task::default()
            },
        ];
        let items = task_completions(&tasks, "kb", "x ", false, true);
        assert_eq!(values(&items), ["x KB-1"]);
        assert_eq!(items[0].label.as_deref(), Some("KB-1  login"));
    }
}
