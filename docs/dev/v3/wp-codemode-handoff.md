# wp-codemode implementation handoff (OIN-9)

## Runtime and permission integration

- `CodemodeTool` and `ToolSearchTool` are registered built-ins. The provider tool
  contract carries JavaScript in `{ "code": "..." }`, not a freeform tool input.
  The runner also accepts `script` for direct execution compatibility.
- QuickJS has a 256 MiB memory limit, interrupt-driven whole-script deadlines,
  two process-wide worker slots (overridable with `PI_CODEMODE_MAX_WORKERS`),
  bounded output, saved overflow logs, images, console output and branch-local
  transactional `codemode-store` custom entries. No Node, filesystem, network,
  module loader or timers are installed in the VM.
- Every `tools.<identifier>` call uses `ToolContext::execute_nested` →
  `SessionToolContext` → `AgentSession::execute_nested_tool` → `NestedHooks` →
  `AgentSession::before_tool_call`. **wp-autonomy must gate this last method**,
  shared with direct calls, after existing execution-only normalization and before
  invoking any tool. Do not gate only the outer `codemode` call.
- Hidden/model-only tools are absent from sandbox discovery. Removed tools are
  resolved against the live registry. Recursive codemode calls are unavailable.
  Sanitized-identifier collisions expose neither ambiguous method.
- Tool discovery loads exact comma-separated names atomically, ignoring the limit;
  keywords use AND matching against case-insensitive names, default five results.
  Previously declared schemas are never replaced or appended twice. Later tools
  remain deferred, including non-MCP tools, until explicitly loaded.
- Core refreshes request declarations from live agent state so a loaded tool can
  execute on the next request inside the same turn. Persisted declaration bytes,
  frozen system bytes and initial context are not rewritten.
- `loaded_deferred_tool_names()` is the public wp-compaction integration point.
  Declaration snapshots already persist loads on the active branch, and
  `rebuild_declarations_after_compaction()` retains loaded available tools while
  dropping removed declarations. The future compaction-state record is not
  implemented here.

## Verification

All commands from repository root with `CARGO_BUILD_JOBS=8`:

- `cargo test -p oino-core --test v3_codemode --test v3_prefix --test v3_reminders --test v3_legacy_replay`: 59 passed.
- `cargo test -p oino-tools --lib --test v3_fs --test v3_exec --test golden_units`: 61 passed.
- `cargo clippy -p oino-tools -p oino-core -- -D warnings`: passed.

The new acceptance file includes the two required named cases plus limit/storage
and only-mode/exposure regressions. It exercises actual built-ins, MCP tools,
next-request declaration loading, shared-hook denial (including a shell write),
partial output, CPU deadline, dead promises, successful-only store commits,
reload/compaction rebuild, catalog updates, output caps and unawaited calls.

## Ownership exceptions

- `crates/oino-tools/Cargo.toml` and root `Cargo.lock`: QuickJS dependency.
- `crates/oino-tools/src/config.rs`: session dispatcher/catalog/store callbacks.
- `crates/oino-core/src/builtin.rs`: bind callbacks to the session, persist storage.
- New acceptance tests and this handoff document. Other core/registration edits
  are explicitly permitted ordered edits in the spec.

## Deviations and remaining verification

- Host model registry and transport currently support chat models only. The VM
  exposes catalog lookup and explicit error results for classifier/image calls;
  it cannot run non-LLM models until a host non-chat runtime is implemented.
  No credentials are exposed to the sandbox and no chat fallback is used.
- Sandbox `searchTools` uses deterministic weighted name/description keyword
  ranking, not pi's BM25 implementation. `tool_search` itself follows the v3
  name-keyword contract.
- Provider input uses the existing JSON tool envelope rather than pi freeform
  tool input. Exposure mode and inline budget read existing raw settings keys
  `codemode.mode` and `codemode.inlineBudget`; no settings migration is applied.
- No real Windows run or full-workspace suite was performed. No WASM extension
  sources changed, so no component rebuild was required.
- The externally referenced `r-qa-backend` audit payload was not supplied or found
  in repository docs. Tests cover nested permission bypass, hidden tools, output
  limits and unknown names, but cannot claim exact audit-input coverage.
