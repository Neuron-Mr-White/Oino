# Round 3: root test targets

## Outcome

PASS for all 16 assigned targets: 94 tests. Full `oino-core`: 268 tests passed, including checkpoint/detach, durability, v3 prefix, legacy replay, autonomy and compaction. No permission, append-only, prefix-cache or data-safety assertion was weakened. No production authorization defaults changed.

## Commits

- `4905f105` — checkpoint canonical paths, consistent prefix on reopen/continuation, durability diagnostics; fixture shell authorization (B/C/E).
- `710b8ae5` — core expectations and session golden (A/B/C/D/F/H).
- `98c10ccd` — agent steering and resource prompt goldens (A/C/D/G).
- `9ba03885` — CLI/client contracts (A/B/C/E/F).

## Failure triage

(a) intended v3 change; (b) regression. Rows covering multiple failing cases name each cause.

| Target | Cause | Classification | Fix |
|---|---|---|---|
| agent / agent_loop_golden | Steering adds trusted follow-up reminder events and messages | (a), G/C | Regenerated event/transcript golden; retained exact assertions |
| CLI / contract | Startup messages now include frozen prompt and cwd/date context; distinct docs-root inputs and timestamps differed | (a), A/C | Give transports identical docs-root input; normalize only timestamp and per-sandbox cwd; compare remaining content and declarations |
| CLI / dist | Frozen prefix is captured before turn event subscription, no mutable sections event; bg_run retired | (a), A/B/C | Inspect startup prefix through RPC get_messages; assert staged docs and task_output declaration |
| CLI / frontend | reserveTokens replaced by tokenLimit | (a), F | Assert 250000 default and invalid-value fallback/diagnostic on tokenLimit |
| CLI / rpc | Read output no longer ends in newline; fresh session has two immutable slots; user shell conservatively High; continuation emitted redundant system declaration | (a), B/C/E; (b), C | Update shape/count; explicit High fixture setting; prefix continuation code fix removes redundant declaration |
| client / hub | Legacy reopen lacked synthetic system in live state, reducing message count; shell fixture denied at Medium | (b), C; (a), E | Always reconstruct live prefix after successful startup; explicitly authorize shell fixture at High, retain no-loss and eviction assertions |
| client / in_process_gui | Atomic write ensures trailing newline | (a), B | Expect newline; retain checkpoint/rewind assertions |
| core / cwd_change | Read formatting and conservatively High user shell | (a), B/E | Update read text; explicitly authorize pwd fixture; retain append-only and provider-prefix assertions |
| core / durability | New image-policy and sent-frontier persistence failed before old barrier, losing barrier diagnostic | (b) | Label both pre-request persistence failures as durability-barrier failures; tests unchanged and fail-closed checks retained |
| core / gui_feeds | Write newline; canonical file_path ignored by checkpoint recorder; sleep floor High blocks detach fixture | (a), B/E; (b), B | Capture file_path with legacy path fallback; test canonical and legacy inputs; explicit High fixture; preserve exact original v0 bytes on rewind |
| core / mode_tools_next_turn | Tests expected declaration removal/replacement despite immutable declaration prefix | (a), C/H | Assert startup declarations persist across mode switches and re-registration; compare frozen provider system message |
| core / runtime_flow | Expanded builtins, read formatting, removed mutable tools/skills prompt sections, frozen declarations, user shell High | (a), A/B/C/D/E/H | Update expected registry and prompt content; preserve builtin retention; explicit High shell fixture |
| core / session_flow | No per-turn system patch events, initial synthetic user slot, declaration descriptions instead of prompt snippets, user-role compaction summary | (a), A/C/F | Update event order, transcript roles, slot indexes, tool declaration and summary assertions |
| core / session_golden | Persisted snapshot/reminder, image-policy/frontier entries replace mutable prompt message | (a), A/C | Refresh fixture; normalize environmental session-start payload and random entry references only; full prefix-byte checks remain in v3_prefix |
| resources / golden | Oino frozen prompt excludes tools/cwd/skills sections | (a), A/C/D | Refresh exact sections/state/text golden |
| resources / loader | Workspace/skill context no longer belongs in immutable system prompt | (a), A/C/D | Assert preamble/docs/addendum and exclusion of context/skill text |

## Code and ownership

Production edits are limited to `crates/oino-core/src/session/{feeds,hooks,mod}.rs`. Other changes are tests/fixtures under agent, CLI (`crates/oino`), client, core and resources. No edits to the concurrent workers' client implementation files or task-service authorization files. The session core files are narrow ownership exceptions needed to correct the regressions above.

## Evidence

Full logs: `/tmp/oino-fix3-root/`.

- `targets-final.log`: exit 0, 94 passed across all 16 assigned targets. Command:
  ```sh
  CARGO_BUILD_JOBS=8 cargo test -p oino-agent --test agent_loop_golden -p oino-cli --test contract --test dist --test frontend --test rpc -p oino-client --test hub --test in_process_gui -p oino-core --test cwd_change --test durability --test gui_feeds --test mode_tools_next_turn --test runtime_flow --test session_flow --test session_golden -p oino-resources --test golden --test loader --no-fail-fast
  ```
- `core-all.log`: `CARGO_BUILD_JOBS=8 cargo test -p oino-core --no-fail-fast`, exit 0, 268 passed.
- `safety.log`: scoped cwd_change/v3_prefix/v3_compaction/v3_legacy_replay/v3_autonomy, exit 0, 85 passed.
- `clippy-scoped.log`: `CARGO_BUILD_JOBS=8 cargo clippy -p oino-agent -p oino-cli -p oino-core -p oino-client -p oino-resources -- -D warnings`, exit 0.
- `git diff --check`: pass. Root target measured 25 GB, below 30 GB GC threshold.

## Risks / open questions

Extra exploratory `--all-targets` clippy was red in existing `crates/oino-agent/tests/v3_guards.rs`: needless_pass_by_value at line 80, await_holding_lock at lines 165/191. These tests were not modified. Requested default strict crate clippy passes.

An intermittent RPC fixture timeout was observed twice in `v2_hello_receives_registry_patches_and_resyncs`: the effect patch arrived before the response and `rpc.ok` consumed it. Subsequent runs passed, including the final gate. Follow-up should use `response_and` for those action/effect pairs; no assertion was removed here.

No full root-workspace gate, Windows execution or GUI QA was run by this worker. Parallel worker changes and pre-existing dirty distribution artifacts were left untouched.

## Recommended next step

Run the supervisor's integrated root workspace gate after all three Round 3 workers commit, preserving the logs above. Release readiness remains separate from this scoped PASS.
