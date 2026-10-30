# Settings

Oino uses pi 1.4.0's settings model and file names, rooted at `~/.oino` instead of
`~/.pi/agent`:

| File | Scope |
|---|---|
| `~/.oino/settings.json` | global |
| `<project>/.oino/settings.json` | project (only read when the project is trusted; project wins) |

Writes are atomic and locked; pi's migrations of old keys are applied on load.

## Core keys (pi)

| Key | Meaning |
|---|---|
| `defaultProvider`, `defaultModel`, `defaultThinkingLevel` | startup model and thinking level (updated when you pick a model) |
| `modelThinkingLevels` | per-model thinking level memory (`"provider/id": "high"`) |
| `enabledModels` | the scoped model list (`--models`) |
| `thinkingBudgets` | token budgets per thinking level |
| `steeringMode`, `followUpMode` | `all` or `one-at-a-time` delivery of queued messages |
| `compaction.enabled`, `compaction.reserveTokens`, `compaction.keepRecentTokens` | automatic compaction |
| `branchSummary.reserveTokens`, `branchSummary.skipPrompt` | summaries when navigating the session tree |
| `retry.enabled`, `retry.maxRetries`, `retry.baseDelayMs`, `retry.maxAgentDelayMs` | automatic retry of transient provider errors |
| `transport`, `httpIdleTimeoutMs`, `cacheWarming`, `showCacheMissNotices` | provider transport and prompt-cache behaviour |
| `images.autoResize`, `images.blockImages` | image handling |
| `shellPath`, `shellCommandPrefix` | the `bash` tool's shell |
| `enableSkillCommands` | expose skills as `/skill:<name>` commands |
| `sessionDir` | where sessions are stored ([sessions.md](sessions.md)) |
| `defaultProjectTrust` | default answer for untrusted projects |
| `packages` | extension package sources (pi packages; see [packages.md](packages.md)) |
| `quietStartup`, `hideThinking`, `theme`, `doubleEscapeAction`, `treeFilter` | TUI preferences |

## Oino keys

| Key | Meaning |
|---|---|
| `extensionSettings.<namespace>` | settings of an extension namespace (e.g. `extensionSettings.permission` for `unipi.workflow`), deep-merged global → project |
| `disabledExtensions` | extension ids that are not loaded |
| `ui.density` | frontend density hint |
| `serve.*` | reserved for `oino serve` (not available yet) |

Extension secrets (API keys entered in extension settings) live in `~/.oino/secrets.json`
(mode `0600`), never in `settings.json`.

## Editing

- TUI: `/settings` edits the core keys above; the namespace rows under them open one page per
  settings namespace (booleans and enums toggle in place, text/number/secret rows open an inline
  editor, `list`/`dict` fields open a row sub-page, `json` fields open `$EDITOR`).
- Desktop: one Settings screen with the agent keys, every extension namespace and the host/app
  settings, rendered by the same form engine.

Settings namespaces use the OSS-2 dialect (`docs/dev/v2/specs/ui-protocol.md` §7). Core also
contributes `oino.*` namespaces (model, compaction, retry, shell, …) describing the pi keys
above, so they appear in the same UI. A stored value that no longer matches its field (type,
enum membership, range, pattern) is ignored in favour of the layer below and reported as a
diagnostic; writing a value that equals the layer below deletes the key instead of storing it.

## Project trust

Project-local files (`.oino/settings.json`, `.oino/extensions`, `.oino/skills`, …) are only
used once the project is trusted. `-a/--approve` trusts it for one run, `-na/--no-approve`
ignores project files for one run. Decisions are stored in `~/.oino/trust.json`.
