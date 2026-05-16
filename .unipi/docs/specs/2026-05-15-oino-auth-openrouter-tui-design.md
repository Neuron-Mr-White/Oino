---
title: "Oino Auth, OpenRouter Provider, and Ratatui Shell"
type: brainstorm
date: 2026-05-15
---

# Oino Auth, OpenRouter Provider, and Ratatui Shell

## Problem Statement

Oino has a headless core runtime, but it cannot yet perform a real interactive model conversation. The next milestone is to start the “miracle”: a Pi-like runtime shell in Rust that can authenticate to one real provider, call it through the existing `StreamProvider` boundary, and show a minimal terminal interface with a message list and input box.

The key design constraint is to preserve the clean core layering already established. Authentication, provider HTTP details, and Ratatui UI must not leak into `oino-agent-loop`. The low-level loop should continue to consume typed `AssistantStreamEvent`s and know nothing about API keys, provider JSON, terminal state, or app commands.

## Context

Current Oino crates:

- `oino-types`: shared model/message/content/stream types.
- `oino-agent-loop`: pure async loop, stream provider trait, tool protocol, lifecycle events.
- `oino-agent`: stateful prompt/queue/abort/subscriber wrapper.
- `oino-session`: append-only sessions and JSONL persistence.
- `oino-env`: execution environment abstraction.
- `oino-harness`: headless runtime binding agent, session, env, hooks, provider/auth placeholders.

Relevant Pi architecture lessons:

- Pi separates credential storage/resolution from provider implementations.
- Pi uses `AuthStorage` plus a model/provider registry; provider adapters ask for credentials instead of owning storage.
- Pi resolves API keys from runtime/CLI override, auth file, environment variables, and custom provider config.
- Pi stores credentials in `~/.pi/agent/auth.json`; OpenRouter maps to env var `OPENROUTER_API_KEY` and auth key `openrouter`.
- Pi custom providers are registered separately from auth. Providers define base URLs, models, request serialization, streaming, and compatibility behavior.
- Pi TUI concepts are componentized, but Oino will be Rust/Ratatui-native rather than copying TypeScript component APIs.

OpenRouter facts for the first provider:

- Base URL: `https://openrouter.ai/api/v1`.
- Chat endpoint: `/chat/completions`.
- Auth header: `Authorization: Bearer <OPENROUTER_API_KEY>`.
- Compatible with OpenAI Chat Completions shape.
- Streaming uses SSE chunks and `[DONE]`.
- Optional app attribution headers: `HTTP-Referer`, `X-OpenRouter-Title`.
- Finish reasons normalize to values like `stop`, `length`, `tool_calls`, `error`.

## Chosen Approach

Add three new library crates plus one binary/runtime crate:

```text
crates/oino-auth
crates/oino-provider-openrouter
crates/oino-tui
crates/oino-app        # binary/runtime wiring, name may become `oino`
```

Dependency direction:

```text
oino-auth                  # generic credential storage/resolution
oino-provider-openrouter -> oino-auth + oino-agent-loop + oino-types
oino-tui                 -> oino-types, later harness-facing app state
oino-app                 -> oino-auth + oino-provider-openrouter + oino-harness + oino-tui
```

Provider code should not live inside auth. Auth may know that provider id `openrouter` maps to env var `OPENROUTER_API_KEY`, but it should not know how OpenRouter HTTP requests work.

## Why This Approach

### Alternative 1 — Put OpenRouter inside `oino-auth`

Rejected. This conflates credentials with provider behavior. It would make auth less reusable when adding Anthropic, OpenAI, Gemini, or local OpenAI-compatible providers. It also violates the desired dependency direction: auth should not depend on HTTP/provider serialization.

### Alternative 2 — Put auth and OpenRouter directly into `oino-harness`

Rejected for now. The harness should remain the generic headless runtime. Provider-specific code in harness would make the first provider special and create future cleanup work.

### Alternative 3 — Single `oino-app` crate for everything

Rejected as the main architecture, though the app crate will wire everything together. Keeping separate auth/provider/tui crates preserves testability and lets future provider and UI work evolve independently.

## Design

## 1. `oino-auth`

Purpose: store and resolve provider credentials without knowing provider HTTP protocols.

Core types:

- `AuthStorage`: async API for loading, saving, deleting credentials.
- `AuthConfig`: location of auth file and optional runtime overrides.
- `AuthCredential`: credential variants.
  - First version: `ApiKey { key: String }`.
  - Future: OAuth credentials with access/refresh/expires.
- `AuthResolver`: resolves provider credential by provider id.
- `ProviderAuthSpec`: maps provider id to env var and auth file key.
- `AuthError`: typed errors for missing credentials, malformed auth file, I/O, permissions, command resolution.

First provider mapping:

```text
provider id: openrouter
auth file key: openrouter
env var: OPENROUTER_API_KEY
```

Resolution order for first version:

1. runtime override, if provided by app/tests
2. auth file entry, e.g. `~/.oino/auth.json`
3. environment variable, e.g. `OPENROUTER_API_KEY`

Auth file shape should intentionally resemble Pi while remaining Oino-owned:

```json
{
  "openrouter": { "type": "api_key", "key": "sk-or-..." }
}
```

Security requirements:

- Create auth file parent directories if needed.
- Write auth file with user-only permissions on Unix where feasible.
- Avoid logging secret values.
- Tests should use temp dirs, not real home directories.

Deferred auth features:

- OAuth/device/browser login flows.
- Shell-command credential resolution.
- Secret manager integrations.
- Token refresh.

## 2. `oino-provider-openrouter`

Purpose: implement the existing `StreamProvider` trait for OpenRouter.

Core types:

- `OpenRouterProvider`: implements `oino_agent_loop::StreamProvider`.
- `OpenRouterConfig`: base URL, optional app title/referer, default model behavior, HTTP timeout.
- `OpenRouterError`: auth, HTTP, serialization, SSE, provider error, unsupported content/tool shape.
- Request/response DTOs internal to the crate.

Responsibilities:

- Use `oino-auth` to resolve provider id `openrouter`.
- Convert `StreamRequest` into OpenRouter Chat Completions JSON.
- Convert Oino `Message` into OpenAI/OpenRouter-compatible messages.
- Convert Oino `ToolDefinition` into OpenRouter function tools.
- Send `stream: true` requests.
- Parse SSE chunks.
- Convert deltas into `AssistantStreamEvent`:
  - text deltas → `TextDelta`
  - reasoning/thinking fields, if available and known → `ThinkingDelta` or ignore with metadata note
  - tool call deltas → `ToolCallDelta`
  - finalized tool calls → `ToolCallDone`
  - usage chunk → `Usage`
  - finish reason → `Done { stop_reason, provider }`
  - provider/API errors → `Error`
  - abort signal → stop request path and return `Aborted` if possible

Stop reason mapping:

```text
stop         -> StopReason::EndTurn
length       -> StopReason::Length
tool_calls   -> StopReason::ToolUse
error        -> StopReason::Error
other/null   -> StopReason::Unknown
```

First-version simplifications:

- Support text-only user/assistant messages first.
- Support tool definitions and tool-call deltas if straightforward, but TUI milestone does not need tools.
- Use one configured model, initially defaulting in app to an OpenRouter model string.
- Keep non-streaming fallback out of scope unless SSE complexity blocks progress.

Testing strategy:

- Unit tests for request serialization from Oino messages/tools.
- Unit tests for SSE chunk parsing using fixture strings.
- Tests for missing auth and provider error response.
- No real OpenRouter API calls in default tests.

## 3. `oino-tui`

Purpose: minimal Ratatui terminal UI that can display conversation state and accept one-line input.

Core types:

- `TuiApp`: owns UI state and event loop entry point.
- `TuiState`: rendered messages, input buffer, status, streaming/working flag, error banner.
- `TuiEvent`: keyboard input, submit prompt, provider response update, quit, resize/tick.
- `MessageView`: display-ready role/content/status projection from Oino messages/events.

Initial screen:

```text
┌ Oino ─────────────────────────────────────────┐
│ user: hello                                   │
│ assistant: hi!                                │
│                                               │
├───────────────────────────────────────────────┤
│ > input box                                   │
└ Enter send • Esc/Ctrl-C quit ─────────────────┘
```

Behavior:

- Render message list above input.
- Render one-line editable input box at bottom.
- Enter submits prompt if input is non-empty.
- Esc or Ctrl-C exits.
- Backspace edits input.
- Basic printable characters append to input.
- While waiting for provider, show a working indicator/status line.
- When the harness returns messages, refresh displayed transcript.

Ratatui implementation notes:

- Use `ratatui` for layout/widgets.
- Use `crossterm` backend for terminal events.
- Keep UI state independent from provider logic.
- App crate supplies async runtime and channel wiring between TUI events and harness calls.

First-version simplifications:

- No markdown rendering yet.
- No scrolling beyond simple last-N visible messages if needed.
- No mouse support.
- No model picker.
- No command palette.
- No session tree UI.

## 4. `oino-app` / binary wiring

Purpose: Pi-like runtime shell that composes auth, OpenRouter provider, harness, session, and TUI.

Responsibilities:

- Build `AuthStorage` using default path `~/.oino/auth.json`.
- Register/construct OpenRouter provider.
- Create a default `Model` such as provider `openrouter`, name from config/env/CLI.
- Create a `SessionManager` for an in-memory or JSONL-backed session.
- Create `HarnessConfig` with OpenRouter stream provider and auth resolver.
- Start Ratatui UI.
- Submit UI prompts to `Harness::prompt`.
- Update TUI message list from harness result.

Configuration for first version:

- `OPENROUTER_API_KEY` or `~/.oino/auth.json`.
- `OINO_MODEL` optional; default can be a known OpenRouter model string.
- Optional OpenRouter attribution env vars later:
  - `OINO_OPENROUTER_REFERER`
  - `OINO_OPENROUTER_TITLE`

Future app features:

- `/login` command to write API keys.
- `/model` picker.
- persisted sessions under `~/.oino/sessions`.
- command mode and keybinding config.
- streaming partial render instead of final response refresh only.

## Data Flow

```text
User types in Ratatui
  -> TuiEvent::SubmitPrompt(text)
  -> app sends Message::user_text(text) to Harness::prompt
  -> Harness runs before_agent_start/context/provider hooks
  -> OpenRouterProvider receives StreamRequest
  -> AuthResolver resolves openrouter credential
  -> provider sends POST /api/v1/chat/completions stream=true
  -> provider parses SSE into AssistantStreamEvent values
  -> agent loop emits message/tool/lifecycle events
  -> harness persists messages and emits SavePoint/Settled
  -> app updates TuiState from returned messages/events
  -> Ratatui redraws message list + input
```

## Error Handling

- Missing API key: provider returns a typed auth error; app shows a clear TUI error: “Missing OpenRouter API key. Set OPENROUTER_API_KEY or add ~/.oino/auth.json.”
- Malformed auth file: app shows auth-file parse error without printing secrets.
- HTTP 401/403: show authentication/permission failure.
- HTTP 429: show rate limit.
- HTTP 5xx/network: show provider/network error.
- SSE parse error: convert into `AssistantStreamEvent::Error` or provider error.
- Terminal setup failure: return error and restore terminal state if already entered.
- Panic avoidance: respect workspace clippy `unwrap_used` and `expect_used` denies.

## Testing

Auth tests:

- resolve runtime override before file before env.
- read/write auth JSON round trip in temp dir.
- missing credentials returns typed error.
- malformed auth file returns typed error.
- auth file permissions best-effort on Unix.

OpenRouter provider tests:

- request serialization for text messages.
- request serialization with tools.
- SSE text delta parsing.
- SSE usage/done parsing.
- tool-call delta parsing if supported in first pass.
- missing auth behavior.
- provider error response mapping.

TUI tests:

- pure `TuiState` input editing behavior.
- submit clears input and emits prompt event.
- message projection from Oino messages.
- render smoke test with fixed terminal size if practical.

App integration tests:

- app wiring with fake auth and fake stream provider.
- no real OpenRouter API calls in default test suite.

Manual smoke test:

```bash
OPENROUTER_API_KEY=... cargo run -p oino-app
```

Expected: terminal opens, user types a prompt, assistant response appears.

## Implementation Checklist

- [x] Task 1 — Add workspace crates and dependency baseline for `oino-auth`, `oino-provider-openrouter`, `oino-tui`, and app/binary wiring. Covered by plan Task 1.
- [x] Task 2 — Implement `oino-auth` API-key storage/resolution with OpenRouter env/auth-file mapping and tests. Covered by plan Tasks 2–3.
- [x] Task 3 — Implement OpenRouter request serialization and SSE parsing fixtures without live API calls. Covered by plan Tasks 4–5.
- [x] Task 4 — Implement `OpenRouterProvider` as `StreamProvider`, including auth resolution, HTTP request, stream parsing, stop reason mapping, and error handling. Covered by plan Task 6.
- [x] Task 5 — Implement minimal Ratatui state, rendering, and keyboard/input behavior independent of provider logic. Covered by plan Task 7.
- [x] Task 6 — Implement app runtime wiring: auth + OpenRouter + harness + session + TUI event loop. Covered by plan Task 8.
- [x] Task 7 — Add docs for auth file format, OpenRouter setup, first TUI usage, and architecture boundaries. Covered by plan Task 9.
- [x] Task 8 — Add automated tests and final quality gate; include optional manual OpenRouter smoke-test instructions. Covered by plan Task 10.

## Open Questions

1. Binary crate name: should the runnable package be `oino`, `oino-app`, or `oino-cli`? Recommendation: package `oino-app` initially, binary name `oino`.
2. Default OpenRouter model: choose a stable default before implementation, or require `OINO_MODEL`/CLI arg. Recommendation: provide a reasonable default but allow env override.
3. First response rendering: final-only response is simpler; true incremental streaming requires events to update TUI while provider chunks arrive. Recommendation: start with final refresh unless implementation naturally exposes chunks through event sink.
4. Auth file location: use `~/.oino/auth.json` or XDG config dir. Recommendation: `~/.oino/auth.json` for Pi-like clarity in this milestone.
5. `/login` command: defer until the basic TUI shell exists, or include API-key prompt now. Recommendation: defer command parser; allow env/auth file first.

## Out of Scope

- OAuth/subscription login.
- Full Pi command system.
- Model picker UI.
- Session tree UI.
- Markdown rendering and syntax highlighting.
- Tool execution UI rendering.
- MCP and plugins.
- Memory database.
- Full JSON Schema validation beyond existing core boundary.
- Real provider calls in automated tests.
