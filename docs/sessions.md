# Sessions

Sessions use **pi session format v3** exactly: one JSON object per line, flat entries with
`type`, 8-hex `id`, `parentId`, ISO timestamps and pi `AgentMessage` shapes. A session file is
a tree: branches are created by forking or navigating back (`/tree`), and every branch can be
resumed.

- Location: `~/.oino/sessions/--<cwd with / replaced by ->--/<timestamp>_<id>.jsonl`
  (`sessionDir` setting, `OINO_SESSION_DIR` or `--session-dir` override it).
- Older Oino session files (`~/.oino/sessions/<uuid>.jsonl`, formats v1/v2) are migrated to
  v3 on first access; the originals move to `~/.oino/sessions/.legacy/`.
- pi's own session files can be opened directly (`--session path.jsonl`) or imported
  (`/import`).

## Command line

| Flag | Effect |
|---|---|
| `-c, --continue` | continue the most recent session in this directory |
| `-r, --resume` | pick a session to resume |
| `--session <path\|id>` | open a session file or partial id |
| `--session-id <id>` | open or create the session with this exact id |
| `--fork <path\|id>` | fork a session into a new file |
| `--no-session` | ephemeral, nothing is written |
| `-n, --name <name>` | set the display name |
| `--export <file> [out.html]` | export a session to HTML and exit |

## In the TUI

`/new`, `/resume`, `/name`, `/session`, `/tree` (navigate and optionally summarise the
abandoned branch), `/fork`, `/clone`, `/compact [instructions]`, `/import <path>`,
`/export [path]`, `/copy`.

## Compaction

When the context fills up (or on `/compact`), older turns are replaced by a summary entry
(pi compaction, including split turns and file tracking). The bundled `unipi.compactor`
replaces the summariser with a lossless, model-free "vcc" summary by default; see
[unipi.md](unipi.md).
