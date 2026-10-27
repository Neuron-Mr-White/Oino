//! pi `printHelp` with Oino substitutions (D13): product name, `~/.oino`, `OINO_*`
//! variables; pi-ai provider variables, MCP and codemode lines are gone (providers are
//! extensions, deviations/modes.md).

use std::fmt::Write as _;

use oino_core::extensions::api::{FlagSpec, FlagType};

/// Extension flag with the extension that registered it.
#[derive(Clone, Debug, PartialEq)]
pub struct ExtensionFlag {
    /// Registering extension (path/id).
    pub extension_path: String,
    /// The flag.
    pub flag: FlagSpec,
}

const OPTIONS: &str = r#"Usage:
  oino [options] [--] [@files...] [messages...]

Commands:
  oino install <source> [-l] [-y] Install an extension package (dir, archive, URL, git+https)
  oino remove <id> [-l]          Remove an installed extension package
  oino uninstall <id> [-l]       Alias for remove
  oino update [--check]          Update Oino itself from the latest release
  oino update <id>|--extensions  Re-install extension packages from their recorded source
  oino list                      List installed and bundled extension packages
  oino config                    Print the settings.json paths
  oino auth <command>            Print credentials or check provider readiness
  oino serve                     Serve sessions to remote clients
  oino cron <command>            Scheduled, unattended runs: serve/install/list/add/run/… (docs/cron.md)
  oino <command> --help          Show help for install/remove/uninstall/update/list/config/auth

Options:
  --provider <name>              Provider to search for --model (requires --model)
  --model <pattern>              Model pattern or ID (supports "provider/id" and optional ":<thinking>")
  --api-key <key>                API key (defaults to env vars)
  --system-prompt <text>         System prompt (default: coding assistant prompt)
  --append-system-prompt <text>  Append text or file contents to the system prompt (can be used multiple times)
  --mode <mode>                  Output mode: text (default), json, rpc, or acp
  --print, -p                    Non-interactive mode: process prompt and exit
  --continue, -c                 Continue previous session
  --resume, -r                   Select a session to resume
  --session <path|id>            Use specific session file or partial UUID
  --session-id <id>              Use exact project session ID, creating it if missing
  --fork <path|id>               Fork specific session file or partial UUID into a new session
  --session-dir <dir>            Directory for session storage and lookup
  --no-session                   Don't save session (ephemeral)
  --name, -n <name>              Set session display name
  --models <patterns>            Comma-separated model patterns for Ctrl+P cycling
                                 Supports globs (anthropic/*, *sonnet*) and fuzzy matching
  --no-tools, -nt                Disable all tools by default (built-in and extension)
  --no-builtin-tools, -nbt       Disable built-in tools by default but keep extension/custom tools enabled
  --tools, -t <tools>            Comma-separated allowlist of tool names or patterns (*) to enable
                                 Only +name/-name entries add to or remove from the defaults
  --exclude-tools, -xt <tools>   Comma-separated denylist of tool names or patterns (*) to disable
  --thinking <level>             Set thinking level: off, minimal, low, medium, high, xhigh, max
  --extension, -e <path>         Load an extension file or builtin:<name> (can be used multiple times)
  --no-extensions, -ne           Disable extension discovery and built-in extensions (explicit -e paths still work)
  --skill <path>                 Load a skill file or directory (can be used multiple times)
  --no-skills, -ns               Disable skills discovery and loading
  --prompt-template <path>       Load a prompt template file or directory (can be used multiple times)
  --no-prompt-templates, -np     Disable prompt template discovery and loading
  --theme <path>                 Load a theme file or directory (can be used multiple times)
  --use-theme <name[/name]>      Set the initial interactive theme for this run
  --no-themes                    Disable theme discovery and loading
  --no-context-files, -nc        Disable AGENTS.md and CLAUDE.md discovery and loading
  --export <file>                Export session file to HTML and exit
  --list-models [search]         List available models (with optional fuzzy search)
  --verbose                      Force verbose startup (overrides quietStartup setting)
  --tui-mode <mode>              TUI mode: fullscreen (default) or regular
  --approve, -a                  Trust project-local files for this run
  --no-approve, -na              Ignore project-local files for this run
  --offline                      Disable startup network operations (same as OINO_OFFLINE=1)
  --remote <url>                 Interactive mode over a remote session (oino://host:port?token=…)
  --agent-dir <dir>              Config directory (same as OINO_AGENT_DIR)
  --                             End option parsing; treat remaining arguments as messages/files
  --help, -h                     Show this help
  --version, -v                  Show version number

Extensions can register additional flags (e.g., --plan from plan-mode extension)."#;

const EXAMPLES: &str = r#"

Examples:
  # Interactive mode
  oino

  # Interactive mode with initial prompt
  oino "List all .ts files in src/"

  # Include files in initial message
  oino @prompt.md @image.png "What color is the sky?"

  # Non-interactive mode (process and exit)
  oino -p "List all .ts files in src/"

  # Prompt beginning with a dash
  oino -p -- "- Summarize these points"

  # Multiple messages (interactive)
  oino "Read package.json" "What dependencies do we have?"

  # Continue previous session
  oino --continue "What did we discuss?"

  # Start a named session
  oino --name "Refactor auth module"

  # Use different model
  oino --provider openai --model gpt-4o-mini "Help me refactor this code"

  # Use model with provider prefix (no --provider needed)
  oino --model openai/gpt-4o "Help me refactor this code"

  # Use model with thinking level shorthand
  oino --model sonnet:high "Solve this complex problem"

  # Limit model cycling to specific models
  oino --models claude-sonnet,claude-haiku,gpt-4o

  # Limit to a specific provider with glob pattern
  oino --models "github-copilot/*"

  # Cycle models with fixed thinking levels
  oino --models sonnet:high,haiku:low

  # Start with a specific thinking level
  oino --thinking high "Solve this complex problem"

  # Read-only mode (no file modifications possible)
  oino --tools read,grep,find,ls -p "Review the code in src/"

  # Disable one tool while keeping the rest available
  oino --exclude-tools ask_question

  # Export a session file to HTML
  oino --export ~/.oino/sessions/--path--/session.jsonl
  oino --export session.jsonl output.html

Environment Variables:
  OINO_AGENT_DIR                   - Config directory (default: ~/.oino)
  OINO_SESSION_DIR                 - Session storage directory (overridden by --session-dir)
  OINO_OFFLINE                     - Disable startup network operations when set to 1/true/yes
  OINO_CACHE_RETENTION             - Prompt cache retention (none, short, long)

Built-in Tool Names:
  read       - Read file contents
  bash       - Execute bash commands
  powershell - Execute PowerShell commands on Windows
  edit       - Edit files with find/replace
  write      - Write files (creates/overwrites)
  grep       - Search file contents (read-only, off by default)
  find       - Find files by glob pattern (read-only, off by default)
  ls         - List directory contents (read-only, off by default)
"#;

/// pi `printHelp(extensionFlags)` text.
#[must_use]
pub fn help_text(extension_flags: &[ExtensionFlag]) -> String {
    let mut text =
        String::from("oino - AI coding assistant with read, bash, edit, write tools\n\n");
    text.push_str(OPTIONS);
    if !extension_flags.is_empty() {
        text.push_str("\nExtension CLI Flags:\n");
        for ExtensionFlag {
            extension_path,
            flag,
        } in extension_flags
        {
            let value = if flag.kind == FlagType::String {
                " <value>"
            } else {
                ""
            };
            let description = if flag.description.is_empty() {
                format!("Registered by {extension_path}")
            } else {
                flag.description.clone()
            };
            let left = format!("  --{}{value}", flag.name);
            let _ = writeln!(text, "{left:<30}{description}");
        }
    }
    text.push_str(EXAMPLES);
    // `console.log` adds the final newline after the template's own.
    text.push('\n');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_extension_flags_padded() {
        let text = help_text(&[ExtensionFlag {
            extension_path: "ext.plan".into(),
            flag: FlagSpec {
                name: "plan".into(),
                kind: FlagType::Boolean,
                default: None,
                description: String::new(),
            },
        }]);
        let expected = format!(
            "extension).\nExtension CLI Flags:\n{:<30}Registered by ext.plan\n\n\nExamples:",
            "  --plan"
        );
        assert!(text.contains(&expected), "{text}");
        assert!(help_text(&[]).contains("extension).\n\nExamples:"));
        assert!(help_text(&[]).starts_with("oino - AI coding assistant"));
    }
}
