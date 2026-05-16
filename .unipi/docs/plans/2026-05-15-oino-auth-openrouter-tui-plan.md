---
title: "Oino Auth, OpenRouter Provider, and Ratatui Shell — Implementation Plan"
type: plan
date: 2026-05-15
workbranch: ""
specs:
  - .unipi/docs/specs/2026-05-15-oino-auth-openrouter-tui-design.md
---

# Oino Auth, OpenRouter Provider, and Ratatui Shell — Implementation Plan

## Overview

Implement Oino's first real interactive shell while preserving the core runtime boundaries. This plan adds a generic auth crate, an OpenRouter provider crate implementing the existing `StreamProvider` trait, a minimal Ratatui UI crate, and a small app/binary crate that wires auth + provider + harness + session + TUI together.

Work will happen directly on `main` per user choice. The low-level `oino-agent-loop` must remain provider/auth/UI-agnostic: OpenRouter HTTP/SSE serialization belongs in `oino-provider-openrouter`, credential storage/resolution belongs in `oino-auth`, and terminal rendering belongs in `oino-tui`/app wiring.

## Decisions Locked By This Plan

- Package `oino-app` will provide binary name `oino`.
- Default auth file path: `~/.oino/auth.json`.
- OpenRouter provider id/auth file key: `openrouter`.
- OpenRouter env var: `OPENROUTER_API_KEY`.
- Auth resolution order: runtime override → auth file → env var.
- First UI renders final refreshed transcript after a prompt; true token-by-token TUI streaming is deferred unless it falls out naturally from existing events.
- `/login`, model picker, markdown rendering, session tree UI, plugins/MCP, and OAuth are out of scope for this first milestone.

## Tasks

- completed: Task 1 — Add workspace crates and dependency baseline
  - Description: Create the auth, OpenRouter provider, TUI, and app crates with the right workspace membership, dependencies, lint inheritance, and minimal compileable public APIs.
  - Dependencies: None.
  - Acceptance Criteria:
    - Root `Cargo.toml` includes `crates/oino-auth`, `crates/oino-provider-openrouter`, `crates/oino-tui`, and `crates/oino-app`.
    - Workspace dependencies include only what is needed for this milestone, likely `reqwest`, `ratatui`, `crossterm`, `dirs`, and existing shared crates.
    - `oino-app` package exposes a binary named `oino`.
    - `cargo check --workspace` succeeds with placeholder implementations.
  - Steps:
    1. Add the four crate directories and `Cargo.toml` files.
    2. Wire workspace dependencies and inherit workspace lints.
    3. Add minimal `src/lib.rs` or `src/main.rs` files for each crate.
    4. Keep dependency direction one-way: provider depends on auth/core; app depends on all; auth does not depend on provider/app/TUI.

- completed: Task 2 — Implement `oino-auth` credential model and storage
  - Description: Build generic API-key storage/resolution that can serve OpenRouter now and future providers later.
  - Dependencies: Task 1.
  - Acceptance Criteria:
    - Public types exist for `AuthConfig`, `AuthCredential`, `ProviderAuthSpec`, `AuthStorage` or equivalent storage/resolver API, and `AuthError`.
    - Auth JSON shape supports `{ "openrouter": { "type": "api_key", "key": "..." } }`.
    - Save creates parent directory and writes user-only permissions on Unix on a best-effort basis.
    - Secrets are not exposed in `Debug`/error messages beyond redacted placeholders.
    - Unit tests cover read/write round trip, missing credentials, malformed auth file, and OpenRouter provider spec mapping.
  - Steps:
    1. Define provider-id/auth-key/env-var mapping with an `openrouter` helper/spec.
    2. Define `AuthCredential::ApiKey` serialization/deserialization using `snake_case` type tagging.
    3. Implement file load/save/delete helpers using Tokio or synchronous std APIs consistently with crate dependencies.
    4. Implement typed errors via `thiserror` without including secret values.
    5. Add temp-dir based tests.

- in-progress: Task 3 — Implement `oino-auth` resolution order and harness adapter
  - Description: Resolve credentials in the chosen order and expose a small adapter usable by the existing `oino-harness::AuthResolver` boundary.
  - Dependencies: Task 2.
  - Acceptance Criteria:
    - Runtime override beats auth file, and auth file beats env var.
    - Env var fallback resolves `OPENROUTER_API_KEY` for provider id `openrouter`.
    - Missing credential returns a typed `MissingCredential`-style error or `None` at the harness adapter boundary, with provider code converting it to a clear provider error.
    - An adapter can produce `oino_harness::AuthResolver` without adding an `oino-harness` dependency to `oino-auth` if avoiding cycles is cleaner; otherwise the dependency direction remains documented and intentional.
    - Tests cover precedence and env fallback without depending on the user's real environment.
  - Steps:
    1. Add runtime override map to `AuthConfig`/resolver.
    2. Implement `resolve(provider_id)` using the locked precedence order.
    3. Add helper to resolve API key text for provider code.
    4. Add optional adapter function in app or provider layer for harness compatibility.
    5. Test precedence with temp files and scoped environment variable setup.

- completed: Task 4 — Implement OpenRouter request DTOs and serialization tests
  - Description: Convert Oino stream requests into OpenRouter/OpenAI-compatible chat-completions JSON without making network calls.
  - Dependencies: Task 1.
  - Acceptance Criteria:
    - Internal DTOs cover `model`, `messages`, `tools`, `tool_choice` if needed, and `stream: true`.
    - Oino `Message::User`, `Message::Assistant`, and `Message::ToolResult` convert to OpenRouter-compatible roles/content for text-first conversations.
    - Oino `ToolDefinition` converts to OpenAI-style `function` tools with name, description, and parameters schema.
    - Unsupported content blocks produce typed serialization errors instead of silently corrupting context.
    - Unit tests assert representative JSON for text messages and tool definitions.
  - Steps:
    1. Create internal request/response DTO module in `oino-provider-openrouter`.
    2. Map `Model.name` to request `model`; require/expect `Model.provider == "openrouter"` in provider path or document fallback behavior.
    3. Convert text content blocks and basic tool calls/tool results.
    4. Return typed errors for image/thinking/custom shapes not supported in first milestone.
    5. Add JSON snapshot-style assertions using `serde_json::json!`.

- completed: Task 5 — Implement OpenRouter SSE parsing and event normalization
  - Description: Parse OpenRouter streaming SSE chunks into Oino `AssistantStreamEvent`s without live API calls.
  - Dependencies: Task 4.
  - Acceptance Criteria:
    - Parser handles `data: {...}` chunks, blank-line SSE separators, and `data: [DONE]`.
    - Text deltas become `AssistantStreamEvent::TextDelta`.
    - Usage chunks become `AssistantStreamEvent::Usage` where present.
    - Finish reasons map to Oino `StopReason`: `stop`, `length`, `tool_calls`, `error`, and unknown/null.
    - Tool-call deltas are parsed into `ToolCallDelta`/`ToolCallDone` if practical; if not fully supported, unsupported cases are documented and tests cover current behavior.
    - Fixture-based tests cover text, done, usage, error-shaped payloads, and at least one partial/multi-chunk case.
  - Steps:
    1. Define response chunk DTOs matching OpenAI-compatible streaming fields.
    2. Implement chunk extraction from SSE text/bytes.
    3. Normalize provider metadata such as provider model/request id where available.
    4. Add stop-reason mapping helper and tests.
    5. Add malformed chunk/error payload tests.

- completed: Task 6 — Implement `OpenRouterProvider` HTTP `StreamProvider`
  - Description: Use auth, request serialization, reqwest streaming, SSE parsing, abort signal checks, and typed error mapping to implement real OpenRouter model calls.
  - Dependencies: Tasks 3, 4, and 5.
  - Acceptance Criteria:
    - `OpenRouterProvider` implements `oino_agent_loop::StreamProvider`.
    - Provider resolves OpenRouter credentials through `oino-auth` and sends `Authorization: Bearer ...`.
    - Provider uses base URL `https://openrouter.ai/api/v1` by default and endpoint `/chat/completions`.
    - Optional attribution headers `HTTP-Referer` and `X-OpenRouter-Title` are configurable.
    - HTTP 401/403/429/5xx and provider error payloads map to clear errors/events without exposing secrets.
    - Abort signal is checked before request and during stream consumption; aborted calls return an abort/error event consistent with existing loop expectations.
    - Tests use a local/mock HTTP server or isolated parser/serialization tests; default suite performs no live OpenRouter calls.
  - Steps:
    1. Define `OpenRouterConfig` and `OpenRouterProvider` constructors.
    2. Build authenticated `reqwest` request with JSON body from `StreamRequest`.
    3. Consume streaming response bytes and feed SSE parser.
    4. Convert parser/provider errors into `LoopResult`/`AssistantStreamEvent::Error` consistently with existing core patterns.
    5. Add mocked HTTP tests for happy path, missing auth, and error response if feasible.

- completed: Task 7 — Implement minimal `oino-tui` state and rendering
  - Description: Build pure UI state, message projection, and a Ratatui render function for a message list plus one-line input box.
  - Dependencies: Task 1.
  - Acceptance Criteria:
    - Public `TuiState`, `MessageView`, and input/action APIs exist independent of provider logic.
    - User/assistant/tool/error-ish messages can be projected from `oino-types::Message` into display lines.
    - Render function draws message area, input box, and status/help line with Ratatui.
    - Input behavior supports printable chars, backspace, enter-submit, Esc/Ctrl-C quit, and non-empty prompt submission.
    - Tests cover input editing, submit-clears-input behavior, message projection, and render smoke test with a fixed backend if practical.
  - Steps:
    1. Define `TuiState` fields: messages, input, status, working flag, optional error banner.
    2. Define pure actions/events such as `InputChanged`, `SubmitPrompt`, `Quit`, `SetMessages`, `SetError`.
    3. Implement message projection from Oino messages to display strings.
    4. Implement Ratatui layout: message panel, input panel, status/help line.
    5. Add pure-state tests before terminal integration.

- unstarted: Task 8 — Implement app runtime wiring and terminal event loop
  - Description: Compose auth, OpenRouter, harness, in-memory session, and TUI into a runnable `oino` binary.
  - Dependencies: Tasks 3, 6, and 7.
  - Acceptance Criteria:
    - `cargo run -p oino-app --bin oino` starts the TUI.
    - Binary reads optional `OINO_MODEL`, `OINO_OPENROUTER_REFERER`, and `OINO_OPENROUTER_TITLE` env vars.
    - Default model is documented and can be overridden by `OINO_MODEL`.
    - Enter submits the current input to `Harness::prompt(Message::user_text(...))` and refreshes displayed messages from harness/session state.
    - Missing API key is shown as a user-facing TUI error: set `OPENROUTER_API_KEY` or add `~/.oino/auth.json`.
    - Terminal raw/alternate-screen state is restored on normal exit and error paths.
    - Tests cover app construction with fake dependencies where practical; live provider smoke remains manual only.
  - Steps:
    1. Choose and document a default OpenRouter model string in app config.
    2. Create auth storage with default path `~/.oino/auth.json`.
    3. Instantiate `OpenRouterProvider` and `HarnessConfig` with in-memory `SessionManager`.
    4. Add crossterm terminal setup/teardown around the TUI loop.
    5. On prompt submit, run harness prompt and update UI state; show spinner/status while waiting.
    6. Handle quit keys and errors gracefully.

- unstarted: Task 9 — Add user/setup documentation
  - Description: Document how to configure OpenRouter auth, run the TUI, and understand the new crate boundaries.
  - Dependencies: Tasks 2, 6, and 8.
  - Acceptance Criteria:
    - README or docs include `OPENROUTER_API_KEY=... cargo run -p oino-app --bin oino` quickstart.
    - Auth file format and path `~/.oino/auth.json` are documented with redacted examples.
    - OpenRouter env vars and default/override model are documented.
    - Architecture docs explain why provider code is separate from auth and why core loop remains pure.
    - Manual smoke-test instructions are clear and marked as requiring a real API key.
  - Steps:
    1. Update root README with quickstart and current limitations.
    2. Add or update docs page for auth/provider/TUI architecture.
    3. Include troubleshooting for missing key, 401/403, 429, and terminal reset issues.
    4. Add manual smoke-test command and expected behavior.

- unstarted: Task 10 — Final quality gate and plan bookkeeping
  - Description: Verify the whole workspace, update plan/spec statuses, and commit completed work.
  - Dependencies: Tasks 1 through 9.
  - Acceptance Criteria:
    - `cargo fmt --all -- --check` passes.
    - `cargo clippy --workspace --all-targets -- -D warnings` passes.
    - `cargo test --workspace` passes with no live OpenRouter calls.
    - `cargo doc --workspace --no-deps` passes.
    - Plan task statuses are updated from `unstarted` to `completed` or accurately marked if deferred/blocked.
    - Work is committed with a descriptive message.
  - Steps:
    1. Run formatting, linting, tests, and docs.
    2. Fix any failures without relaxing workspace lint standards.
    3. Update this plan's statuses and any implementation notes.
    4. Commit the implementation on `main`.

## Sequencing

1. Start with crate scaffolding (Task 1) so all later work has stable package boundaries.
2. Build auth first (Tasks 2–3), because provider runtime depends on credential resolution.
3. Build provider serialization and parsing separately (Tasks 4–5), before network I/O, so most behavior is fixture-testable.
4. Implement the real OpenRouter `StreamProvider` (Task 6) once auth and parser pieces are stable.
5. Build TUI state/rendering independently (Task 7), in parallel conceptually with provider work but after scaffolding.
6. Wire the app/binary (Task 8) only after auth/provider/TUI APIs are usable.
7. Document user setup (Task 9), then run final quality gates and commit (Task 10).

Dependency graph:

```text
Task 1
├─ Task 2 ─ Task 3 ─┐
├─ Task 4 ─ Task 5 ─┼─ Task 6 ─┐
└─ Task 7 ──────────┘          ├─ Task 8 ─ Task 9 ─ Task 10
                               └──────────┘
```

## Risks

- **Reqwest streaming/SSE shape mismatch:** OpenRouter may include provider-specific fields not covered by initial DTOs. Mitigate with tolerant deserialization and fixture tests.
- **Tool-call streaming complexity:** OpenAI-compatible tool-call deltas can arrive fragmented. Implement robust accumulation if feasible; otherwise document first-version limitations and keep text chat working.
- **Existing harness auth boundary is string/placeholder-shaped:** `oino-harness::AuthResolver` currently returns `Option<String>`. Keep app/provider auth usage clear and avoid forcing provider-specific auth into harness internals.
- **Terminal teardown bugs:** Raw mode/alternate-screen must be restored on errors. Keep setup/teardown small and test pure UI state separately.
- **Clippy lint strictness:** Workspace denies `unwrap`/`expect`. Use typed errors and explicit propagation from the start.
- **Live API instability/cost:** Automated tests must not call OpenRouter. Keep live smoke tests manual and opt-in.
- **Scope creep toward full Pi UI:** Defer command palette, model picker, markdown rendering, sessions UI, `/login`, OAuth, MCP, plugins, and memory DB until after the first working shell.

## Manual Smoke Test Target

After implementation, with a real OpenRouter key:

```bash
OPENROUTER_API_KEY=sk-or-... cargo run -p oino-app --bin oino
```

Expected result:

1. TUI opens with an empty message panel and bottom input box.
2. User types a prompt and presses Enter.
3. Status indicates the app is waiting/working.
4. Assistant response appears in the message panel.
5. Esc or Ctrl-C exits and restores the terminal.
