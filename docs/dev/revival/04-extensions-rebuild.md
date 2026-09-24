# P4: Extension Rebuild

Goal: cut steps and cost per task, following FrontierHarness lessons, and move optional
functionality out of core onto the P2 host. Each item is an independent extension unless
marked host.

## P4.0 Headless mode (host, prerequisite for P5)

`oino run [-p "<prompt>"] [--json] [--model provider:model] [--max-turns N] [--session <path>|--no-session]`

- `-p` runs one prompt non-interactively and prints the final assistant text.
- `--json` streams `AgentEvent`s as JSON lines (delta-only, P1) plus a final summary line:
  turns, usage totals, cost, token-weighted cache hit rate, stop reason.
- Exit codes: 0 done, 1 error, 2 aborted or cap hit.
- Stdin prompt supported (`-p -`).

## P4.1 `oino.budget` (turn, cost and time caps)

Problem: agents spend the most on tasks they cannot solve. Exo's 51-step cap made it
cheapest overall.

- Uses P1 `finish_turn` to return `End` when any cap trips: `max_turns`, `max_cost_usd`,
  `max_wall_secs`, `max_consecutive_tool_errors`.
- Before ending, it may request one final turn (`Continue`) with a steering message
  asking for a summary of progress and remaining work (setting
  `summary_on_cap`, default on).
- Defaults are off in interactive mode and set in headless mode by flags.
- Emits a transcript notice and records a `Usage`-free custom entry with the cap reason.

## P4.2 `oino.cache` (warming and miss diagnostics)

- **Warming** (port of pi 0.86 behavior):
  - Warm only when the model declares a `prompt_cache` lifetime for the active retention
    tier.
  - Warm only when the estimated avoided cache-miss cost is at least $0.05.
  - Two triggers: a long tool run approaching the cache lifetime, and optional idle time
    (setting `cacheWarming: off | tools | tools+idle`, default `tools`).
  - A warm request is the current transcript with `max_output = 1`. Its usage is recorded
    as a `Usage { kind: "cache_warm" }` session entry and never enters context.
  - Fires the `cache_warming_decision` event so other extensions can veto.
- **Miss diagnostics:**
  - Host records a per-request prefix fingerprint: a list of
    `(block kind, sha256 of canonical bytes)` for the system message, each tool, and each
    message.
  - When the reported cache read is below 50% of the previous request's prompt size,
    diff the fingerprints and show the first diverging block ("tool `grep` definition
    changed", "system section project/AGENT.md changed", "provider dropped cache after
    idle 7m").
  - Shown as notices when `showCacheMissNotices` is on, and in `/session`.

## P4.3 Turn-efficient tools

Fewer, stronger tool calls lower both steps and tokens.

- Core `read` (host): accept `paths: [..]` with optional per-path line ranges; return one
  result with per-file sections. Single `path` stays valid.
- Core `edit` (host): accept `edits: [{old, new}]` applied atomically per file; reject
  ambiguous matches with the match count and line numbers.
- Extension `oino.search`: `grep` (ripgrep semantics via the `grep` crate inside WASM
  over WASI preopens, capped output) and `find` (glob). Read-only filesystem permission.
- All tool outputs are capped (default 30k chars) with an explicit truncation footer
  telling the model how to fetch more.

## P4.4 `oino.presets`

Named presets change prompt sections and the active tool set through `sync_system`
(P1.4), so switching is an append, not a prefix rewrite.

- `minimal`: core tools only, short base prompt.
- `standard`: default.
- `thorough`: adds a planning/verification section and `oino.search`.
- Selected with `/preset <name>` or `--preset` in headless mode; persisted per project.

## P4.5 Deferred tools: `oino.tool-search`

- Extension tools can be declared `deferred: true` in their manifest. Deferred tools are
  not in the initial system message.
- A `tool_search` tool returns matching deferred tool summaries and loads the chosen
  ones via `added_tool_names`, so the host appends a `System { tools_added }` at that
  point. Providers with native deferral keep them out of the cached prefix.

## P4.6 Built-in ports

| Built-in | Target | Notes |
| --- | --- | --- |
| router | WASM provider (P3) | host keeps Docker bootstrap/onboarding |
| ralph-loop | WASM | loop via `finish_turn` + follow-up queue; state via kv + session entries |
| vcc (compaction) | WASM | uses the new compaction entry shape and retain-none; hooks `session_before_compact` |
| notify | WASM | `agent_settled` hook + `host.notify` |
| mode-sandbox | host | enforces tool policy in `before_tool_call`; stays native for trust |
| ask-user | host | TUI modal contract |
| footer-status | host | TUI rendering contract |
| craft-skill | resource-only package | no runtime |

## Acceptance

- Each extension has unit tests in its guest crate plus a host integration test through
  the fixture harness.
- Headless `--json` summary numbers match `/usage` for the same session.
- Budget: a faux provider that never stops is halted at `max_turns`, with one summary
  turn.
- Cache diagnostics: an injected tool-definition change produces the correct first
  diverging block.
