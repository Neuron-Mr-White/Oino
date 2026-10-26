//! Shell-command parsing shared by the write guard, the reminders and the badges (unipi
//! `guard.ts` `shellSegments`/`kanboardInvocations`, `commands.ts` `tokenizeArgs`).
//!
//! The parsing mirrors unipi's regular expressions exactly, including their backtracking
//! quirks, so a command line is classified the same way it was in unipi.

/// The binary the agent calls (`oino-kanboard`, optionally `.exe`).
pub const BINARY: &str = "oino-kanboard";

/// Every subcommand the CLI understands (`crates/oino-kanboard/src/cli.rs` `Command`,
/// kebab-case as clap spells it). Anything else — a typo like `done`, a bare binary with no
/// subcommand — is the binary's own usage error; the guard neither charges nor blocks it.
pub const KNOWN_SUBCOMMANDS: &[&str] = &[
    "project",
    "add",
    "list",
    "show",
    "move",
    "note",
    "attach",
    "attachments",
    "edit",
    "link",
    "unlink",
    "order",
    "claim-next",
    "start",
    "finish",
    "next",
    "reap",
    "queue",
    "unqueue",
    "chain",
    "search",
    "release",
    "set-run",
    "duplicate",
    "archive-sweep",
    "serve",
    "settings",
    "rotate-token",
    "status",
    "stop",
    "validate",
];

/// Subcommands that never write to the board.
const READONLY: &[&str] = &[
    "list",
    "show",
    "attachments",
    "next",
    "chain",
    "search",
    "status",
];

/// Global flags that take a value; `--json` is the only valueless one.
const GLOBAL_VALUE_FLAGS: &[&str] = &["--actor", "--project", "--gate", "--session"];

/// Wrappers that keep the wrapped word in command position.
const COMMAND_PREFIXES: &[&str] = &["exec", "command", "env"];

/// Splits a command line into words the way a shell would: whitespace separates, a token may
/// start with a quoted span, and a quote inside a word is literal (so `it's fine` keeps its
/// apostrophe). Inside double quotes a backslash escapes the next character.
#[must_use]
pub fn tokenize_args(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut words = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if c.is_whitespace() {
            if started {
                words.push(std::mem::take(&mut current));
            }
            current.clear();
            started = false;
            index += 1;
            continue;
        }
        if (c == '"' || c == '\'') && !started {
            let quote = c;
            index += 1;
            started = true;
            while index < chars.len() && chars[index] != quote {
                if quote == '"' && chars[index] == '\\' && index + 1 < chars.len() {
                    index += 1;
                }
                current.push(chars[index]);
                index += 1;
            }
            index += 1;
            continue;
        }
        current.push(c);
        started = true;
        index += 1;
    }
    if started {
        words.push(current);
    }
    words
}

/// End (exclusive) of the quoted span opening at `start`, emulating unipi's
/// `(["'])(?:\\.|(?!\1).)*\1` (JS `.` never matches a newline; greedy with backtracking).
fn quoted_span_end(chars: &[char], start: usize) -> Option<usize> {
    let quote = chars[start];
    // Depth-first search in the regex engine's preference order: escape first, then a plain
    // character, then closing.
    let mut stack: Vec<(usize, u8)> = vec![(start + 1, 0)];
    while let Some((pos, stage)) = stack.pop() {
        match stage {
            0 => {
                stack.push((pos, 1));
                if pos + 1 < chars.len() && chars[pos] == '\\' && chars[pos + 1] != '\n' {
                    stack.push((pos + 2, 0));
                }
            }
            1 => {
                stack.push((pos, 2));
                if pos < chars.len() && chars[pos] != quote && chars[pos] != '\n' {
                    stack.push((pos + 1, 0));
                }
            }
            _ => {
                if pos < chars.len() && chars[pos] == quote {
                    return Some(pos + 1);
                }
            }
        }
    }
    None
}

/// `text` with every quoted span replaced by `fill(span_len)` characters per char.
fn mask_quotes(chars: &[char], fill: char) -> Vec<char> {
    let mut out = Vec::with_capacity(chars.len());
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if (c == '"' || c == '\'')
            && let Some(end) = quoted_span_end(chars, index)
        {
            out.extend(std::iter::repeat_n(fill, end - index));
            index = end;
            continue;
        }
        out.push(c);
        index += 1;
    }
    out
}

/// `text` with every quoted span replaced by `""` (unipi's redirect pre-pass).
#[must_use]
pub fn collapse_quotes(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        if (c == '"' || c == '\'')
            && let Some(end) = quoted_span_end(&chars, index)
        {
            out.push_str("\"\"");
            index = end;
            continue;
        }
        out.push(c);
        index += 1;
    }
    out
}

/// Splits a command line into shell segments on `&&`, `||`, `;`, `|` and newlines — ignoring
/// separators inside quoted spans (the mask keeps the original length, so the cut positions
/// map back onto the input exactly).
#[must_use]
pub fn shell_segments(command: &str) -> Vec<String> {
    let chars: Vec<char> = command.chars().collect();
    let masked = mask_quotes(&chars, ' ');
    let mut out = Vec::new();
    let mut at = 0;
    let mut index = 0;
    while index < masked.len() {
        let c = masked[index];
        let next = masked.get(index + 1).copied();
        let width = match (c, next) {
            ('&', Some('&')) | ('|', Some('|')) => 2,
            (';' | '|' | '\n', _) => 1,
            _ => 0,
        };
        if width == 0 {
            index += 1;
            continue;
        }
        out.push(chars[at..index].iter().collect());
        index += width;
        at = index;
    }
    out.push(chars[at..].iter().collect());
    out
}

/// One `oino-kanboard` invocation inside a shell command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invocation {
    /// First positional after the binary name (`""` when absent).
    pub sub: String,
    /// Everything after the subcommand.
    pub args: Vec<String>,
}

/// `VAR=value` (unipi `/^[A-Za-z_][A-Za-z0-9_]*=/`).
#[must_use]
pub fn is_assignment(word: &str) -> bool {
    let Some(eq) = word.find('=') else {
        return false;
    };
    let name = &word[..eq];
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// A token naming the binary (bare name or path, optionally `.exe`; unipi's suffix match).
#[must_use]
pub fn is_binary_token(token: &str) -> bool {
    token.ends_with(BINARY) || token.ends_with(&format!("{BINARY}.exe"))
}

/// Every `oino-kanboard` invocation inside a shell command line. A token counts only when it
/// is positioned like a command: the first word of its segment, past leading `VAR=value`
/// assignments and the `exec`/`command`/`env` wrappers. `which oino-kanboard` or
/// `find -name "oino-kanboard"` merely mention the binary and are not invocations.
#[must_use]
pub fn invocations(command: &str) -> Vec<Invocation> {
    let mut out = Vec::new();
    for segment in shell_segments(command) {
        let tokens = tokenize_args(&segment);
        let mut head = 0;
        while head < tokens.len()
            && (is_assignment(&tokens[head]) || COMMAND_PREFIXES.contains(&tokens[head].as_str()))
        {
            head += 1;
        }
        if head >= tokens.len() || !is_binary_token(&tokens[head]) {
            continue;
        }
        let rest = &tokens[head + 1..];
        let mut cursor = 0;
        while cursor < rest.len() {
            let arg = rest[cursor].as_str();
            if arg == "--json" {
                cursor += 1;
                continue;
            }
            if GLOBAL_VALUE_FLAGS.contains(&arg) {
                cursor += 2;
                continue;
            }
            if GLOBAL_VALUE_FLAGS
                .iter()
                .any(|flag| arg.starts_with(&format!("{flag}=")))
            {
                cursor += 1;
                continue;
            }
            break;
        }
        out.push(Invocation {
            sub: rest.get(cursor).cloned().unwrap_or_default(),
            args: rest
                .get(cursor + 1..)
                .map(<[String]>::to_vec)
                .unwrap_or_default(),
        });
    }
    out
}

impl Invocation {
    /// Read-only means: no writes, and the board does not change.
    #[must_use]
    pub fn is_readonly(&self) -> bool {
        if READONLY.contains(&self.sub.as_str()) {
            return true;
        }
        let first = self.args.first().map(String::as_str);
        match self.sub.as_str() {
            "project" => matches!(first, Some("list" | "show")),
            "settings" => first != Some("set"),
            "validate" => !self.args.iter().any(|a| a == "--fix"),
            _ => false,
        }
    }

    /// First non-flag positional (the task id for task commands).
    #[must_use]
    pub fn first_positional(&self) -> Option<&str> {
        self.args
            .iter()
            .map(String::as_str)
            .find(|arg| !arg.starts_with('-'))
    }

    /// `move <ID> blocked` (the free own-claim close).
    #[must_use]
    pub fn is_move_blocked(&self) -> bool {
        self.sub == "move"
            && self.args.iter().any(|a| a == "blocked")
            && self.first_positional().is_some()
    }

    /// `edit --strategy …` / `--plan …` — strategy labels died with the runner.
    #[must_use]
    pub fn is_removed_edit(&self) -> bool {
        self.sub == "edit"
            && self.args.iter().any(|a| {
                a == "--strategy"
                    || a.starts_with("--strategy=")
                    || a == "--plan"
                    || a.starts_with("--plan=")
            })
    }

    /// The subcommand exists in the CLI.
    #[must_use]
    pub fn is_known(&self) -> bool {
        KNOWN_SUBCOMMANDS.contains(&self.sub.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn subs(command: &str) -> Vec<String> {
        invocations(command).into_iter().map(|i| i.sub).collect()
    }

    #[test]
    fn tokenizes_like_a_shell() {
        assert_eq!(
            tokenize_args(r#"add "a b" 'c d' it's "x\"y""#),
            ["add", "a b", "c d", "it's", "x\"y"]
        );
        assert_eq!(tokenize_args("  "), Vec::<String>::new());
        assert_eq!(tokenize_args(r#"a "" b"#), ["a", "", "b"]);
    }

    #[test]
    fn splits_segments_ignoring_quoted_separators() {
        assert_eq!(
            shell_segments(r"a && b || c; d | e"),
            ["a ", " b ", " c", " d ", " e"]
        );
        assert_eq!(shell_segments("a\nb"), ["a", "b"]);
        assert_eq!(
            shell_segments(r#"note X-1 "a; b && c" && list"#),
            [r#"note X-1 "a; b && c" "#, " list"]
        );
        // Unclosed quotes do not mask.
        assert_eq!(shell_segments(r#"echo "a; b"#), [r#"echo "a"#, " b"]);
    }

    #[test]
    fn cut_positions_keep_content_intact() {
        let cmd = "oino-kanboard note KB-1 'one | two' | cat";
        let segs = shell_segments(cmd);
        assert_eq!(segs[0], "oino-kanboard note KB-1 'one | two' ");
        assert_eq!(segs[1], " cat");
    }

    #[test]
    fn finds_command_position_invocations() {
        assert_eq!(
            subs("FOO=1 env /opt/oino/oino-kanboard --actor agent --project p start KB-1"),
            ["start"]
        );
        assert_eq!(subs("exec oino-kanboard.exe list"), ["list"]);
        assert_eq!(subs("which oino-kanboard"), [] as [std::string::String; 0]);
        assert_eq!(
            subs(r#"find . -name "oino-kanboard""#),
            [] as [std::string::String; 0]
        );
        assert_eq!(subs("oino-kanboard --json --gate=done show KB-2"), ["show"]);
        assert_eq!(subs("oino-kanboard"), [""]);
        assert_eq!(
            subs("oino-kanboard show A-1 && oino-kanboard finish A-1 --comment 'x'"),
            ["show", "finish"]
        );
    }

    #[test]
    fn classifies_reads_and_writes() {
        let inv = |c: &str| invocations(c).remove(0);
        for read in [
            "list",
            "show A-1",
            "attachments A-1",
            "next",
            "chain A-1",
            "search x",
            "status",
        ] {
            assert!(
                inv(&format!("oino-kanboard {read}")).is_readonly(),
                "{read}"
            );
        }
        for write in [
            "add x",
            "move A-1 todo",
            "note A-1 x",
            "start A-1",
            "finish A-1",
        ] {
            assert!(
                !inv(&format!("oino-kanboard {write}")).is_readonly(),
                "{write}"
            );
        }
        assert!(inv("oino-kanboard project list").is_readonly());
        assert!(inv("oino-kanboard project show").is_readonly());
        assert!(!inv("oino-kanboard project archive s").is_readonly());
        assert!(!inv("oino-kanboard project rebind s --root /x").is_readonly());
        assert!(inv("oino-kanboard settings").is_readonly());
        assert!(inv("oino-kanboard settings show").is_readonly());
        assert!(!inv("oino-kanboard settings set a b").is_readonly());
        assert!(inv("oino-kanboard validate").is_readonly());
        assert!(!inv("oino-kanboard validate --fix").is_readonly());
    }

    #[test]
    fn move_blocked_and_removed_edit() {
        let inv = |c: &str| invocations(c).remove(0);
        assert!(inv("oino-kanboard move A-1 blocked --comment x").is_move_blocked());
        assert!(!inv("oino-kanboard move A-1 done").is_move_blocked());
        assert!(!inv("oino-kanboard list").is_move_blocked());
        assert!(!inv("oino-kanboard move A-1 todo").is_move_blocked());
        assert!(!inv("oino-kanboard note A-1 blocked").is_move_blocked());
        assert!(inv("oino-kanboard edit A-1 --strategy goal").is_removed_edit());
        assert!(inv("oino-kanboard edit A-1 --plan=yes").is_removed_edit());
        assert!(!inv("oino-kanboard add x --strategy goal").is_removed_edit());
        assert_eq!(
            inv("oino-kanboard note -x A-1 t").first_positional(),
            Some("A-1")
        );
        assert_eq!(inv("oino-kanboard --json list").first_positional(), None);
    }

    #[test]
    fn known_subcommands_match_the_cli() {
        for sub in [
            "start",
            "finish",
            "release",
            "archive-sweep",
            "rotate-token",
            "validate",
        ] {
            assert!(KNOWN_SUBCOMMANDS.contains(&sub));
        }
        assert!(!invocations("oino-kanboard done A-1")[0].is_known());
    }

    #[test]
    fn collapses_quotes_for_the_redirect_pass() {
        assert_eq!(collapse_quotes(r#"grep "a > b" f"#), r#"grep "" f"#);
    }
}
