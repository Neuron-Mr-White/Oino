# oino.budget

Run caps for agent runs. Enabled but inert by default — configure caps via the
extension config (`~/.oino/extensions/oino.budget/config.json`) or headless
flags:

- `max_turns` — max agent turns (`--max-turns` CLI flag also applies; this is
  the extension-level variant for interactive runs)
- `max_cost_usd` — total assistant usage cost (`--max-cost <usd>`)
- `max_wall_secs` — wall-clock seconds (`--max-wall <secs>`)
- `max_consecutive_tool_errors` — consecutive `is_error` tool results
  (`--max-tool-errors <n>`)
- `summary_on_cap` (default `true`) — inject one "wrap up now" user turn
  before ending the run

On a cap the extension emits a notice, records a `budget_cap` session entry,
and ends the run (`end_run`), which maps to `stop_reason: aborted` / exit
code 2 headless.
