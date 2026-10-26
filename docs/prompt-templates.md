# Prompt templates

Prompt templates are Markdown files invoked as slash commands (pi 1.4.0 semantics):
`~/.oino/prompts/review.md` becomes `/review`. Front matter may set `description` and
`argument-hint`; the body may use `$1`, `$2`, `$@` and `${@:2}` for arguments.

| Location | Scope |
|---|---|
| bundled extension prompts | extensions |
| `~/.oino/prompts/` | global |
| `<project>/.oino/prompts/` | project (trusted projects only) |
| `--prompt-template <path>` | this run |

`-np/--no-prompt-templates` disables discovery.
