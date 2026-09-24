# Oino Revival Specs

Status: draft for review (2026-09-24). Owner: Oino maintainers.

Oino was last active on 2026-05-31, roughly in step with pi 0.78. Upstream pi
(`earendil-works/pi`) is now at 0.87.1. This folder specifies how Oino catches up on the
core SDK, moves models and optional functionality into WASM extensions, and rebuilds
extensions using lessons from FrontierHarness Eval v1.0.

## Decisions (settled)

| Topic | Decision |
| --- | --- |
| WASM engine | Wasmtime with the component model and WIT interfaces. The JSON-v1 ABI and `FixtureWasmModule` are retired. |
| Provider split | Full provider adapters live in WASM: wire format, SSE parsing, auth shaping, cache hints, usage normalization. Core exposes brokered streaming HTTP only. |
| Transcript model | Mirror pi: system prompt and tool declarations are carried by `System` messages inside the transcript and replayed. |
| Core tools | `read`, `write`, `edit`, `bash` stay in core, like pi. New tools ship as extensions. |

## Baseline (2026-09-24)

- `cargo build --workspace`: pass (rustc 1.98.1).
- `cargo test --workspace`: 468 passed, 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings`: fails, 5 lints, all in
  `crates/oino-auth/src/lib.rs` (`unnecessary_map_or` :756, `ptr_arg` :1271,
  `unwrap_used` :1698/:1717/:1730).

## Gap matrix (pi 0.87.1 vs Oino)

| Area | pi | Oino today | Spec |
| --- | --- | --- | --- |
| Prompt/tools in transcript | `SystemMessage { content, sections, toolsAdded, toolsRemoved }`, replay + collapse | `StreamRequest.system_prompt` + `tools` resent whole; prompt not persisted | P1 |
| Provider cache hints | `cache_control`, `prompt_cache_key`, session affinity, `cacheRetention`, per-model lifetimes | usage parsed only; no hints sent | P1 (interim), P3 |
| Deferred tool loading | `ToolResultMessage.addedToolNames`, native `tool_addition` / `additional_tools` | none | P1, P3 |
| Context edits | `context_edit` entries (omit/replace without rewriting history) | none | P1 |
| Compaction | kept-boundary id, retain-none, `tokensBefore`, per-model reserve/keep | `replaces: Vec<OinoId>` | P1 |
| Loop hooks | `prepareRequest`, `finishTurn`, actionable `turn_end`, `peekQueuedMessages`, block `terminate` | `should_stop_after_turn`, `Allow/Block(String)` | P1 |
| Types | `max` thinking, `Usage.reasoning`, tool-result usage, thinking signatures, delta-only updates | none of these | P1 |
| Extension events | ~40 | ~20 `HookEventKind`s, stringly payloads | P1 |
| Cache warming | cost-aware, `cache_warming_decision` | none | P4 |
| WASM runtime | n/a (pi uses trusted jiti TS) | manifests + broker, no engine | P2 |
| Headless mode | `-p`, `--mode json`, RPC | none | P4 |
| Remote sessions (`protocol/client/server`), `durable`, `telemetry`, `chord` | new packages | none | Out of scope for this revival; revisit after P5 |

## Phases

1. [P0 + P1: core parity](01-core-parity.md). Clippy fix, types, transcript, loop hooks,
   session v2, extension events, interim cache hints.
2. [P2: WASM host](02-wasm-host.md). Wasmtime component host, WIT package
   `oino:extension`, capability gating, guest SDK.
3. [P3: providers as WASM](03-provider-extensions.md). OpenAI-compatible, OpenAI
   Responses and Anthropic adapters plus the router, ported to components; core provider
   crate removed.
4. [P4: extension rebuild](04-extensions-rebuild.md). Headless mode, budget caps, cache
   warming and diagnostics, turn-efficient tools, presets, built-in ports.
5. [P5: benchmark](05-benchmark.md). Run Oino through FrontierHarness.

Phases are ordered by dependency: P3 needs P1 types and the P2 host; P4 needs P1 hooks and
P2; P5 needs P4 headless mode. P1 and P2 can proceed in parallel after P0.

## FrontierHarness takeaways that shape these specs

FrontierHarness v1.0 (Runta, 2026-09-01) ran 9 harnesses, 12 configurations, 30 tasks
(21 Terminal-Bench, 9 DeepSWE), all on Kimi K3 via Fireworks, one attempt per cell.

- **Pass rate is mostly noise at n=30.** One task is 3.3 points; the field spans 50.0%
  to 66.7%. Do not optimize prompts against the leaderboard pass rate.
- **Cost is the signal.** Median cost per pass spans $1.05 to $18.34. Cost is steps
  times per-step price; caching only lowers the price.
- **Pi's edge is fewer turns and a stable prefix.** On the hardest task, Pi passed in 90
  turns, 98.2% cache hit, $2.50. Codex took 187 turns for $5.97; Claude Code took 381
  turns at 15.7% cache hit for $64.36.
- **Caps cut losses.** Exo is cheapest partly because it stops at 51 steps.
- **Presets matter.** DSH Creator vs Minimal changed pacing and outcome with the same
  harness, model and runtime. DSH PTC (programmatic tool calling) cost more here.
- **Implicit caches punish unstable prefixes.** Any byte change early in the request (a
  reloaded prompt, reordered tools, a timestamp) turns the rest of the request into
  fresh input.

## Conventions for all phases

- Follow `AGENT.md`: `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace` before each commit.
- New dependencies: prefer versions published at least 7 days ago; pin exact minor.
- No real provider calls in tests. Use `FauxStream` and recorded SSE fixtures.
- Each phase ends with a user-facing doc update under `docs/` and an implementation note.
