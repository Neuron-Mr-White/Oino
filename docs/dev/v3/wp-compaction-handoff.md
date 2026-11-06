# wp-compaction / OIN-5 Round 2

## Outcome
Implemented anchored append-only v3 state, delta summarization, durable sent-frontier protection, common triggers and feature settings. Board left read-only per worker rules.

## Findings
- P1-5: manager and boundary drafts reject sent/off-branch edits. Context hooks receive only the unsent tail; accepted content replacements are persisted before the durable frontier advances. Structural transforms are refused. Failed-frontier persistence fails the request closed.
- P1-7: commit validates existing inclusive user-turn anchors, raw branch index, branch membership and pending tool pairs. Invalid reload records preserve the prior valid view and expose projection errors. Legacy records remain readable; all new compaction writes use compaction_state. Source entries remain append-only.
- Adaptive §5.1 with conditional paragraph, §5.3/5.4 prompts and deterministic §5.5 Oino envelope replace classic compaction. Previous TODO suffix is stripped, latest tool list appended. 6k soft cap uses the exact IMPORTANT line. Instruction clamp, 4k reserve, omitted marker and 90/75/50 fitting ladder implemented. Empty output resamples then one final main-model request; 180-second idle guard retained.
- vcc/llm selection remains in the compactor. VCC uses common state persistence and clamps its anchor to the earlier host/VCC user turn. Percentage-trigger execution is disabled in favor of common host usage thresholds. Model usage thresholds exclude output, include cache reads/writes, and suppress repeats. Overflow requires progress and retries only the model continuation.
- Declaration rebuild and fresh provider cache namespace follow committed state. Existing skill-registry SessionCompact handler reannounces skills; core appends loaded deferred-tool reminder. Immutable slots are unchanged.

## Evidence
- `cargo test -p oino-core --test v3_compaction`: 18 passed. Includes audit exact send/edit/send and nonexistent-anchor inputs, off-branch/tool-result anchors, invalid reload fallback, crash-before-commit, deterministic bytes, side-effect-once overflow, three deltas, soft cap, retention, suppression, manual-only blocking, and one escalation.
- Core prefix/legacy/compaction fixtures: 47 + 6 + 5 + 6 passed. Core library: 91 passed.
- Full oino-session tests: 84 passed, 1 ignored. Settings library: 19 passed. Compactor library: 29 passed.
- Strict clippy: oino-core, oino-session, oino-settings, unipi-compactor passed.
- Compaction Settings Vitest: 2 passed. `cd apps/unipi && npx tsc --noEmit`: passed on final retry.
- `cargo xtask wasm -p unipi-compactor`: rebuilt component. Only compactor dist artifacts/checksum keys staged.

## Ownership exceptions
Narrow changes in session context/manager/migration exports and existing tests for state projection/frontier, core context hooks/runner/host/settings registration, settings module export, and prefix/old compaction test expectations. No zvec or Superagent source edits. Concurrent edits in shared files were already committed by their owners before staging.

## Risks / open questions
- Model catalog has no dedicated default-compaction-limit/max-input fields. Resolver supports a model-default argument, but runtime currently supplies None and clamps to contextWindow minus maxTokens. Summarization currently uses the session main model, as the existing core host did; no separate compactor-model picker was present to preserve.
- Retained images are persisted as file references/excerpts, not reloaded image payloads. Previously persisted image references survive subsequent states.
- Frontend settings were typechecked and unit tested, but no browser screenshot or Windows execution was performed.
- Disk-write failure during the append itself is governed by existing JSONL torn-tail recovery. The crash-before-summary-commit path writes no compaction state.

## Recommended next step
Run the independent gpt-sol backend audit against this commit, especially persisted transformed-tail bytes and invalid-state fallback. Review the model-catalog limitations before release approval.
