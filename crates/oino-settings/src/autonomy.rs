//! Fail-closed autonomy policy shared by the host and workflow extension.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Shell risk and execution ceiling, ordered from least to most permissive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Read-only commands.
    Low,
    /// Reversible development commands (the default).
    #[default]
    Medium,
    /// All commands.
    High,
}
impl Level {
    /// Actionable name in refusal messages.
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::Low => "Low",
            Self::Medium => "Medium",
            Self::High => "High",
        }
    }
}

/// Rule effect. Deny and ask take precedence over every allow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effect {
    /// Reject.
    Deny,
    /// Require interactive approval.
    Ask,
    /// Permit, subject to the risk ceiling.
    Allow,
}
/// Applicable rule scope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Scope {
    /// All projects.
    Global,
    /// One stable project identity.
    Project {
        /// Project identity, never inferred from command text.
        #[serde(rename = "projectId")]
        project_id: String,
    },
}
/// One versioned permission rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    /// Unique identity.
    pub id: String,
    /// Applicability.
    pub scope: Scope,
    /// Exact tool name or `*`.
    pub tool: String,
    /// Subject glob, or exact canonical command for interactive grants.
    pub pattern: String,
    /// Outcome.
    pub effect: Effect,
    /// Only an interactive project-scoped exact shell approval may set this.
    #[serde(default)]
    pub interactive_above_ceiling: bool,
}
/// Preserved old deny rule.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyDeny {
    /// Tool name or `*`.
    pub tool: String,
    /// Subject glob.
    pub pattern: String,
}
/// Validated permission rules envelope.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Permissions {
    /// Currently only 1 is supported.
    pub version: u32,
    /// Explicit rules.
    pub rules: Vec<Rule>,
    /// Legacy denies never become allows.
    #[serde(default)]
    pub legacy_deny: Vec<LegacyDeny>,
    /// Fallback for tools without matching rules.
    pub default: Effect,
}
impl Default for Permissions {
    fn default() -> Self {
        Self {
            version: 1,
            rules: Vec::new(),
            legacy_deny: Vec::new(),
            default: Effect::Allow,
        }
    }
}
impl Permissions {
    /// Reject malformed or unsupported envelopes before use.
    ///
    /// # Errors
    /// Invalid version, duplicate ids, scopes or patterns.
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("unsupported permissions version".into());
        }
        let mut ids = std::collections::HashSet::new();
        for rule in &self.rules {
            if rule.id.trim().is_empty()
                || !ids.insert(&rule.id)
                || rule.tool.is_empty()
                || rule.pattern.is_empty()
            {
                return Err("invalid permission rule id, tool or pattern".into());
            }
            if matches!(&rule.scope, Scope::Project { project_id } if project_id.trim().is_empty())
            {
                return Err("permission projectId is empty".into());
            }
            if rule.interactive_above_ceiling
                && (rule.effect != Effect::Allow
                    || !matches!(rule.scope, Scope::Project { .. })
                    || !matches!(rule.tool.as_str(), "bash" | "powershell")
                    || canonical_command(&rule.pattern).as_deref() != Ok(rule.pattern.as_str()))
            {
                return Err("invalid exact interactive permission grant".into());
            }
        }
        if self
            .legacy_deny
            .iter()
            .any(|r| r.tool.is_empty() || r.pattern.is_empty())
        {
            return Err("invalid legacy deny".into());
        }
        Ok(())
    }

    /// Resolve precedence without weakening the ceiling. Interactive grants are ignored unattended.
    #[must_use]
    pub fn resolve(
        &self,
        project: &str,
        tool: &str,
        subjects: &[String],
        interactive: bool,
    ) -> (Effect, bool) {
        let matches = |r: &&Rule| {
            let scoped = match &r.scope {
                Scope::Global => true,
                Scope::Project { project_id } => project_id == project,
            };
            scoped
                && (r.tool == "*" || r.tool == tool)
                && (!r.interactive_above_ceiling || interactive)
                && subjects.iter().any(|s| {
                    if r.interactive_above_ceiling {
                        r.pattern == *s
                    } else {
                        glob(&r.pattern, s)
                    }
                })
        };
        let applicable: Vec<_> = self.rules.iter().filter(matches).collect();
        if applicable.iter().any(|r| r.effect == Effect::Deny) {
            return (Effect::Deny, false);
        }
        if self.legacy_deny.iter().any(|r| {
            (r.tool == "*" || r.tool == tool)
                && subjects
                    .iter()
                    .any(|s| glob(&r.pattern.to_lowercase(), &s.to_lowercase()))
        }) {
            return (Effect::Deny, false);
        }
        if applicable.iter().any(|r| r.effect == Effect::Ask) {
            return (Effect::Ask, false);
        }
        if applicable.iter().any(|r| r.effect == Effect::Allow) {
            return (
                Effect::Allow,
                applicable.iter().any(|r| r.interactive_above_ceiling),
            );
        }
        (self.default, false)
    }
}

/// Case-sensitive subject glob (`*` and `?` only). Exact approvals never use this function.
#[must_use]
pub fn glob(pattern: &str, subject: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = subject.chars().collect();
    let (mut i, mut j, mut star) = (0, 0, None);
    while j < s.len() {
        if i < p.len() && (p[i] == '?' || p[i] == s[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = Some((i, j));
            i += 1;
        } else if let Some((a, b)) = star {
            i = a + 1;
            j = b + 1;
            star = Some((a, j));
        } else {
            return false;
        }
    }
    p[i..].iter().all(|c| *c == '*')
}

/// Parse the global default only. Legacy full/auto settings never elevate it.
#[must_use]
pub fn default_level(global: &serde_json::Map<String, Value>) -> Level {
    global
        .get("autonomy")
        .and_then(|v| v.get("defaultLevel"))
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default()
}

// Preserve the exact spelling of every shell token. Only whitespace *between* tokens
// is canonicalized; in particular quotes, escapes, operators and literal newlines stay.
#[derive(Debug)]
struct Token {
    raw: String,
    word: Option<String>,
    start: usize,
    end: usize,
}

#[allow(
    clippy::too_many_lines,
    reason = "single recursive shell lexer state machine"
)]
fn lex(input: &str, depth: usize) -> Result<(Vec<Token>, Level), String> {
    if depth > 32 {
        return Err("shell nesting limit".into());
    }
    let c: Vec<char> = input.chars().collect();
    let (mut i, mut tokens, mut floor) = (0, Vec::new(), Level::Low);
    while i < c.len() {
        if matches!(c[i], ' ' | '\t' | '\r') {
            i += 1;
            continue;
        }
        let start = i;
        if matches!(c[i], ';' | '&' | '|' | '\n' | '>' | '<')
            && !(matches!(c[i], '<' | '>') && c.get(i + 1) == Some(&'('))
        {
            let ch = c[i];
            i += 1;
            if c.get(i) == Some(&ch) && ch != '\n' {
                i += 1;
            }
            if matches!(ch, '<' | '>') && c.get(i) == Some(&'&') {
                i += 1;
            }
            tokens.push(Token {
                raw: c[start..i].iter().collect(),
                word: None,
                start,
                end: i,
            });
            continue;
        }
        let mut word = String::new();
        let mut quote = None;
        while i < c.len() {
            let ch = c[i];
            if quote.is_none()
                && (ch.is_whitespace() || matches!(ch, ';' | '&' | '|' | '>' | '<'))
                && !(matches!(ch, '<' | '>') && c.get(i + 1) == Some(&'('))
            {
                break;
            }
            if ch == '\\' && quote != Some('\'') {
                let next = *c.get(i + 1).ok_or("trailing escape")?;
                if next == '\n' {
                    return Err("line continuation is not canonicalizable".into());
                }
                word.push(next);
                i += 2;
                continue;
            }
            if matches!(ch, '\'' | '"') && (quote.is_none() || quote == Some(ch)) {
                if quote == Some(ch) {
                    quote = None;
                } else {
                    quote = Some(ch);
                }
                i += 1;
                continue;
            }
            if quote != Some('\'')
                && ((ch == '$' && c.get(i + 1) == Some(&'('))
                    || (quote.is_none() && matches!(ch, '<' | '>') && c.get(i + 1) == Some(&'(')))
            {
                let end = substitution_end(&c, i + 2)?;
                let nested: String = c[i + 2..end].iter().collect();
                floor = floor.max(bash_floor_depth(&nested, depth + 1));
                word.push_str("__substitution__");
                i = end + 1;
                continue;
            }
            if quote != Some('\'') && ch == '`' {
                let mut end = i + 1;
                while end < c.len() && c[end] != '`' {
                    if c[end] == '\\' {
                        return Err("escaped backtick".into());
                    }
                    end += 1;
                }
                if end == c.len() {
                    return Err("unclosed backtick".into());
                }
                floor = floor.max(bash_floor_depth(
                    &c[i + 1..end].iter().collect::<String>(),
                    depth + 1,
                ));
                word.push_str("__substitution__");
                i = end + 1;
                continue;
            }
            if quote.is_none() && matches!(ch, '(' | ')' | '{' | '}' | '#') {
                return Err("unsupported shell syntax".into());
            }
            // Parameter expansion stays in the word text (`$`); `simple_bash` fails
            // closed when it reaches a command name or an option-sensitive command.
            word.push(ch);
            i += 1;
        }
        if quote.is_some() {
            return Err("unbalanced shell quote".into());
        }
        if i == start {
            return Err("unknown shell syntax".into());
        }
        tokens.push(Token {
            raw: c[start..i].iter().collect(),
            word: Some(word),
            start,
            end: i,
        });
    }
    Ok((tokens, floor))
}
fn substitution_end(c: &[char], start: usize) -> Result<usize, String> {
    let (mut i, mut nesting, mut quote) = (start, 1usize, None);
    while i < c.len() {
        let ch = c[i];
        if ch == '\\' && quote != Some('\'') {
            i += 2;
            continue;
        }
        if matches!(ch, '\'' | '"') && (quote.is_none() || quote == Some(ch)) {
            if quote == Some(ch) {
                quote = None;
            } else {
                quote = Some(ch);
            }
        } else if quote.is_none() {
            if ch == '(' {
                nesting += 1;
            }
            if ch == ')' {
                nesting -= 1;
                if nesting == 0 {
                    return Ok(i);
                }
            }
        }
        i += 1;
    }
    Err("unclosed substitution".into())
}

/// Canonical parsed shell token serialization for persistent *exact* grants.
///
/// # Errors
/// Unsupported, ambiguous or unbalanced syntax cannot be persisted.
pub fn canonical_command(input: &str) -> Result<String, String> {
    let (tokens, _) = lex(input, 0)?;
    if tokens.is_empty() {
        return Err("empty command".into());
    }
    for (i, token) in tokens.iter().enumerate() {
        if token.word.is_none()
            && !matches!(
                token.raw.as_str(),
                ";" | "&" | "&&" | "||" | "|" | "\n" | ">" | ">>" | "<" | ">&" | "<&"
            )
        {
            return Err("unsupported persistent shell syntax".into());
        }
        if token.word.is_none()
            && matches!(token.raw.as_str(), "&&" | "||" | "|")
            && (i == 0 || tokens.get(i + 1).is_none_or(|t| t.word.is_none()))
        {
            return Err("incomplete shell chain".into());
        }
        if token.word.is_none()
            && matches!(token.raw.as_str(), ">" | ">>" | "<" | ">&" | "<&")
            && tokens.get(i + 1).is_none_or(|t| t.word.is_none())
        {
            return Err("incomplete shell redirect".into());
        }
    }
    let mut canonical = String::new();
    for (i, token) in tokens.iter().enumerate() {
        if i > 0
            && tokens[i - 1].end < token.start
            && tokens[i - 1].raw != "\n"
            && token.raw != "\n"
        {
            canonical.push(' ');
        }
        canonical.push_str(&token.raw);
    }
    Ok(canonical)
}

/// Conservative syntax-aware bash floor. Unknown syntax and commands are High.
#[must_use]
pub fn bash_floor(input: &str) -> Level {
    bash_floor_depth(input, 0)
}
fn bash_floor_depth(input: &str, depth: usize) -> Level {
    let Ok((tokens, mut risk)) = lex(input, depth) else {
        return Level::High;
    };
    let mut words = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if let Some(word) = &t.word {
            words.push(word.clone());
            i += 1;
            continue;
        }
        match t.raw.as_str() {
            ";" | "&" | "&&" | "||" | "|" | "\n" => {
                risk = risk.max(simple_bash(&words, depth));
                words.clear();
            }
            ">" | ">>" | "<" | ">&" | "<&" => {
                let Some(target) = tokens.get(i + 1).and_then(|t| t.word.as_deref()) else {
                    return Level::High;
                };
                // Descriptors are lexed as words; removing a standalone number can
                // only increase ambiguity, never turn an unknown executable into low.
                if i > 0
                    && tokens[i - 1].end == t.start
                    && words
                        .last()
                        .is_some_and(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_digit()))
                {
                    words.pop();
                }
                if matches!(t.raw.as_str(), ">" | ">>") && target != "/dev/null" {
                    risk = risk.max(Level::Medium);
                }
                if matches!(t.raw.as_str(), ">&" | "<&") && !matches!(target, "0" | "1" | "2" | "-")
                {
                    return Level::High;
                }
                i += 1;
            }
            _ => return Level::High,
        }
        i += 1;
    }
    risk.max(simple_bash(&words, depth))
}
/// Leading `NAME=value` words are environment assignments, not the executable.
fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && !name.starts_with(|c: char| c.is_ascii_digit())
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

/// Owner decision (round 4, Droid parity): the model's declared `riskLevel` is
/// the primary signal. This floor only (a) recognises read-only commands as Low,
/// (b) raises commands it positively knows to be destructive, privileged,
/// opaque or option-sensitive to High, and (c) gives every other executable a
/// Medium floor because the harness cannot prove it read-only.
#[allow(
    clippy::too_many_lines,
    reason = "one flat command table reads best as a single match"
)]
fn simple_bash(w: &[String], depth: usize) -> Level {
    let w = &w[w.iter().take_while(|a| is_assignment(a)).count()..];
    let Some(bin) = w.first().map(String::as_str) else {
        return Level::Low;
    };
    // An expanded command name can be anything.
    if bin.contains('$') || bin.contains("__substitution__") {
        return Level::High;
    }
    // Wrappers run the rest of the words as a command.
    match bin {
        "time" | "nohup" | "command" => return simple_bash(&w[1..], depth),
        "nice" => {
            let skip = match w.get(1).map(String::as_str) {
                Some("-n") => 3,
                Some(a) if a.starts_with('-') => 2,
                _ => 1,
            };
            return simple_bash(w.get(skip..).unwrap_or_default(), depth);
        }
        "timeout" => {
            let rest = w[1..].iter().skip_while(|a| a.starts_with('-')).skip(1);
            return simple_bash(&rest.cloned().collect::<Vec<_>>(), depth);
        }
        "mise" if matches!(w.get(1).map(String::as_str), Some("exec" | "x")) => {
            // Tool selection before `--`; the command after it is what runs.
            return match w.iter().position(|a| a == "--") {
                Some(i) => simple_bash(&w[i + 1..], depth).max(Level::Medium),
                None => Level::Medium,
            };
        }
        _ => {}
    }
    // Option-sensitive commands: an expanded word could become any option.
    if matches!(bin, "git" | "find" | "fd" | "rg" | "env" | "xargs" | "rm")
        && w.iter()
            .any(|a| a.contains('$') || a.contains("__substitution__"))
    {
        return Level::High;
    }
    // No basename matching: a local executable named `ls` is not the trusted ls.
    let arg = |i| w.get(i).map_or("", String::as_str);
    if (bin == "rg" && w.iter().any(|a| a == "--pre" || a.starts_with("--pre=")))
        || (bin == "fd"
            && w.iter().any(|a| {
                matches!(a.as_str(), "--exec" | "--exec-batch")
                    || a.starts_with("--exec=")
                    || a.starts_with("--exec-batch=")
                    // Short flags may be bundled and their command argument attached.
                    || (a.starts_with('-') && !a.starts_with("--") && a.contains(['x', 'X']))
            }))
        || (bin == "git"
            && w.iter().any(|a| {
                matches!(a.as_str(), "--ext-diff" | "--textconv" | "--output")
                    || a.starts_with("--output=")
            }))
    {
        return Level::High;
    }
    match bin {
        "bash" | "sh" | "zsh" if w.len() == 3 && arg(1) == "-c" => {
            bash_floor_depth(arg(2), depth + 1)
        }
        "echo" | "printf" | "pwd" | "ls" | "cat" | "head" | "tail" | "wc" | "grep" | "rg"
        | "fd" | "stat" | "du" | "df" | "whoami" | "uname" | "which" | "true" | "false" => {
            Level::Low
        }
        "find"
            if !w.iter().any(|a| {
                a.starts_with("-exec")
                    || a.starts_with("-ok")
                    || a.starts_with("-delete")
                    || matches!(a.as_str(), "-fprint" | "-fprint0" | "-fprintf" | "-fls")
            }) =>
        {
            Level::Low
        }
        "cd" | "pushd" | "popd" | "export" | "unset" | "set" | "type" | "date" | "id"
        | "hostname" | "ps" | "tree" | "file" | "sort" | "uniq" | "cut" | "tr" | "diff"
        | "basename" | "dirname" | "realpath" | "readlink" | "sha256sum" | "md5sum"
        | "less" | "more" | "jq"
            if !w.iter().any(|a| a.starts_with("-o") || a.starts_with("--output")) =>
        {
            Level::Low
        }
        "env" => {
            let rest: Vec<String> =
                w[1..].iter().skip_while(|a| is_assignment(a)).cloned().collect();
            if rest.first().is_some_and(|a| a.starts_with('-')) {
                // -S/--split-string re-parse a string; other options are rare.
                Level::High
            } else {
                simple_bash(&rest, depth)
            }
        }
        "git" => git_floor(&w[1..]),
        "rm" if w.iter().any(|a| {
            a == "--recursive"
                || a == "--force"
                || (a.starts_with('-') && !a.starts_with("--") && a.contains(['r', 'R', 'f']))
        }) =>
        {
            Level::High
        }
        "chmod" | "chown" | "chgrp"
            if w.iter().any(|a| {
                a == "--recursive" || (a.starts_with('-') && !a.starts_with("--") && a.contains('R'))
            }) =>
        {
            Level::High
        }
        "curl" | "wget"
            if w.iter().any(|a| {
                matches!(
                    a.as_str(),
                    "-X" | "--request" | "-d" | "-F" | "-T" | "--form" | "--upload-file"
                        | "--post-data" | "--post-file" | "--method"
                ) || a.starts_with("--data")
                    || a.starts_with("--request=")
                    || a.starts_with("--method=")
            }) =>
        {
            Level::High
        }
        // Shells, eval and sourcing run opaque code; privilege and system control.
        "bash" | "sh" | "zsh" | "dash" | "ksh" | "fish" | "eval" | "exec" | "source" | "."
        | "xargs" | "sudo" | "su" | "doas" | "pkexec" | "dd" | "shred" | "truncate"
        | "wipefs" | "fdisk" | "parted" | "mount" | "umount" | "kill" | "pkill"
        | "killall" | "reboot" | "shutdown" | "poweroff" | "halt" | "crontab" | "iptables"
        | "ssh" | "scp" | "sftp" | "rsync" | "nc" | "ncat" | "socat" | "rmdir"
        // find with -exec/-ok/-delete/-fprint* (the read-only form matched above).
        | "find" => Level::High,
        b if b.starts_with("mkfs") => Level::High,
        "systemctl" | "launchctl" | "service"
            if !matches!(arg(1), "status" | "is-active" | "is-enabled" | "list-units" | "list") =>
        {
            Level::High
        }
        "systemctl" | "launchctl" | "service" => Level::Low,
        // Builds, tests, package managers, project scripts and unknown programs:
        // the declared riskLevel decides above this floor.
        _ => Level::Medium,
    }
}

fn git_floor(w: &[String]) -> Level {
    let mut i = 0;
    while let Some(a) = w.get(i) {
        match a.as_str() {
            // Configuration and exec-path options can run arbitrary programs.
            o if o.starts_with("-c")
                || o.starts_with("--config-env")
                || o.starts_with("--exec-path") =>
            {
                return Level::High;
            }
            "-C" | "--git-dir" | "--work-tree" | "--namespace" => i += 2,
            o if o.starts_with('-') => i += 1,
            _ => break,
        }
    }
    let sub = w.get(i).map_or("", String::as_str);
    let args = w.get(i + 1..).unwrap_or_default();
    let has = |f: &[&str]| args.iter().any(|a| f.contains(&a.as_str()));
    match sub {
        "" | "status" | "diff" | "log" | "show" | "blame" | "rev-parse" | "ls-files" | "grep"
        | "describe" | "shortlog" | "ls-tree" | "cat-file" | "rev-list" | "merge-base" => {
            Level::Low
        }
        "push" | "reset" | "clean" | "rebase" | "filter-branch" | "filter-repo" | "restore"
        | "update-ref" | "reflog" | "gc" | "prune" | "replace" => Level::High,
        "checkout" | "switch" if has(&["--", ".", "-f", "--force", "--discard-changes"]) => {
            Level::High
        }
        "branch" | "tag" if has(&["-d", "-D", "--delete", "-f", "--force", "-M"]) => Level::High,
        "stash"
            if args
                .first()
                .is_some_and(|a| matches!(a.as_str(), "drop" | "clear")) =>
        {
            Level::High
        }
        "worktree"
            if args
                .first()
                .is_some_and(|a| matches!(a.as_str(), "remove" | "prune")) =>
        {
            Level::High
        }
        _ => Level::Medium,
    }
}

/// Conservative PowerShell floor, including nested expressions and invocation syntax.
#[must_use]
pub fn powershell_floor(input: &str) -> Level {
    let lower = input.to_ascii_lowercase();
    // PowerShell expansion/invocation has different grammar from bash. Unsupported
    // scriptblocks, interpolations, escaping and invocation operators fail closed.
    if lower.contains(['`', '$', '&', '{', '}', '(', ')']) && !lower.contains("$null") {
        return Level::High;
    }
    let Ok((tokens, mut risk)) = lex(input, 0) else {
        return Level::High;
    };
    let mut words = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if let Some(word) = &t.word {
            words.push(word.to_ascii_lowercase());
        } else {
            match t.raw.as_str() {
                ";" | "|" | "\n" | "&&" | "||" => {
                    risk = risk.max(simple_powershell(&words));
                    words.clear();
                }
                ">" | ">>" => {
                    let Some(target) = tokens.get(i + 1).and_then(|t| t.word.as_deref()) else {
                        return Level::High;
                    };
                    if i > 0
                        && tokens[i - 1].end == t.start
                        && words
                            .last()
                            .is_some_and(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_digit()))
                    {
                        words.pop();
                    }
                    if !target.eq_ignore_ascii_case("nul") && !target.eq_ignore_ascii_case("$null")
                    {
                        risk = risk.max(Level::Medium);
                    }
                    i += 1;
                }
                _ => return Level::High,
            }
        }
        i += 1;
    }
    // `$null` is a sink, not an unknown expression; the lexer marks it High. Only
    // strip this marker when all dollar occurrences were redirect sinks.
    if lower.contains('$') {
        return powershell_null_floor(input);
    }
    risk.max(simple_powershell(&words))
}
fn powershell_null_floor(input: &str) -> Level {
    let replaced = input.replace("$null", "NUL").replace("$NULL", "NUL");
    if replaced.contains('$') {
        Level::High
    } else {
        powershell_floor(&replaced)
    }
}
fn simple_powershell(w: &[String]) -> Level {
    let Some(bin) = w.first().map(String::as_str) else {
        return Level::Low;
    };
    match bin {
        "get-childitem" | "get-content" | "get-location" | "get-item" | "get-process"
        | "get-date" | "write-output" | "echo" | "ls" | "pwd" => Level::Low,
        "set-content" | "out-file" | "new-item" | "copy-item" | "move-item" => Level::Medium,
        "git" | "cargo" | "npm" | "pnpm" | "yarn" => simple_bash(w, 0),
        // Opaque code, other shells, deletion, process/system control and remoting.
        "iex"
        | "invoke-expression"
        | "invoke-command"
        | "start-process"
        | "pwsh"
        | "powershell"
        | "cmd"
        | "remove-item"
        | "rm"
        | "del"
        | "erase"
        | "rd"
        | "ri"
        | "rmdir"
        | "clear-content"
        | "stop-process"
        | "kill"
        | "restart-computer"
        | "stop-computer"
        | "set-executionpolicy"
        | "enter-pssession"
        | "new-pssession"
        | "invoke-webrequest"
        | "iwr"
        | "invoke-restmethod"
        | "irm"
        | "start-job"
        | "register-scheduledtask"
        | "set-itemproperty"
        | "remove-itemproperty"
        | "."
        | "sudo"
        | "runas" => Level::High,
        b if b.starts_with("format-") && b != "format-table" && b != "format-list" => Level::High,
        // Builds, tests and unknown programs: the declared riskLevel decides above this.
        _ => Level::Medium,
    }
}
