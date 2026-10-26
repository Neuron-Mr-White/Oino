# Environment variables

| Variable | Meaning |
|---|---|
| `OINO_AGENT_DIR` | configuration directory (default `~/.oino`; same as `--agent-dir`) |
| `OINO_SESSION_DIR` | session storage directory (overridden by `--session-dir`) |
| `OINO_OFFLINE` | `1`/`true`/`yes`: no startup network operations (dynamic model lists); same as `--offline` |
| `OINO_CACHE_RETENTION` | prompt cache retention: `none`, `short`, `long` (`PI_CACHE_RETENTION` is honoured too) |
| `OINO_BUNDLED` | `0`: do not load the components bundled into the binary (tests, debugging) |
| provider keys | `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, … ([providers.md](providers.md)) |

## Exported to tools

Commands run by the `bash` tool (and extension processes) see the current session:

| Variable | Alias | Value |
|---|---|---|
| `OINO_SESSION_ID` | `PI_SESSION_ID` | session id |
| `OINO_SESSION_FILE` | `PI_SESSION_FILE` | session file path |
| `OINO_PROVIDER` | `PI_PROVIDER` | provider of the selected model |
| `OINO_MODEL` | `PI_MODEL` | selected model id |
| `OINO_REASONING_LEVEL` | `PI_REASONING_LEVEL` | thinking level |

The `PI_*` aliases keep skills written for pi working unchanged.

## Kanboard

`oino-kanboard` and `unipi.kanboard` use `OINO_KANBOARD_HOME` (default
`~/.oino/state/unipi.kanboard`), `OINO_KANBOARD_PROJECT`, `OINO_KANBOARD_SESSION`,
`OINO_KANBOARD_ACTOR` and related variables; see `oino-kanboard --help`.
