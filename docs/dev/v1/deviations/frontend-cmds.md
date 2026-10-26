# Deviations: frontend-cmds

Owner: `frontend-cmds` (the `_oino/*` frontend command surface in `oino-core` / `oino-client`
/ `oino-rpc`, its TUI side, ACP `--model`, package manager and self-update in `crates/oino`).
Format per README §5: section, change, reason, impact.

## Command surface (modes §4)

All commands are `OinoCommand` variants, served by `InProcessBackend` and therefore by
`oino --mode rpc` (the RPC server forwards to the backend) and by `RpcBackend`. Typed calls for
every backend: `oino_client::FrontendApi` (blanket impl over `SessionBackend`).

| Command | Request | Response `data` | Core API |
|---|---|---|---|
| `_oino/get_completions` | `{text, cursor}` (cursor = character offset) | `{groups: [{source, from, items: [{value, label?, description?, tag?, tone?, sortTier?}]}]}` | `AgentSession::completions` |
| `_oino/ui_action` | `{panelId, actionId, value?}` | — | `AgentSession::ui_action` |
| `_oino/editor_event` | `{eventId, intent, text, tokenId?}` | `{consumed}` | `AgentSession::editor_event` |
| `_oino/invoke_shortcut` | `{shortcutId}` | — | `AgentSession::invoke_shortcut` |
| `_oino/get_shortcuts` | — | `{shortcuts: [{id, defaultKeys, description, extensionId}]}` | `ExtensionRunner::shortcuts` |
| `_oino/get_settings_schema` | — | `{namespaces}` | `ExtensionSettings::schema` |
| `_oino/get_extension_settings` | `{namespace, scope?: effective\|global\|project}` | `{values}` (secrets masked) | `ExtensionSettings::values` |
| `_oino/set_extension_setting` / `_oino/unset_extension_setting` | `{namespace, key, value?, scope}` | — | `ExtensionSettings::set` |
| `_oino/get_settings` | `{key?}` | `{settings}` or `{value}` | merged core settings |
| `_oino/set_setting` | `{key, value?, scope}` | — | `SettingsManager::set` |
| `_oino/service_call` | `{target?, name, payload?}` | the service's result | `AgentSession::service_call` (caller `oino.frontend`) |
| `_oino/auth_providers` | — | `{providers: [{id, name, oauth, configured, stored?, env}]}` | `AgentSession::auth_providers` |
| `_oino/login` | `{provider}` | — (after the flow finished) | `AgentSession::login` |
| `_oino/logout` | `{provider}` | — | `AgentSession::logout` |
| `_oino/set_api_key` | `{provider, key}` | — | `AgentSession::set_api_key` |

## Contract differences

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | modes §4 `_oino/invoke_shortcut {id}` | The field is `shortcutId`. | `id` is the RPC correlation id of every command (pi framing); the shortcut id would be swallowed. | Clients send `{"type":"_oino/invoke_shortcut","shortcutId":"…"}`. |
| 2 | ui-protocol §7.3 | Completion groups carry `from` (character offset): an accepted item replaces `text[from..cursor]`. Core fills it for its own groups and defaults it to the token start for `complete-input` groups that omit it. Argument completion returns only the argument value (pi `getArgumentCompletions`). | The protocol did not say what an item replaces. | Frontends replace a range, not "the current token". |
| 3 | ui-protocol §7.3 | `@` paths: a `.gitignore`-aware walk of the cwd (depth 8, ≤ 20 000 entries, ≤ 50 results, fuzzy, prefix matches first); `~/` and absolute prefixes list one directory level. | No persistent core file index. | Large trees may miss deep files. |
| 4 | ui-protocol §6.1 | `ui_action` goes to the panel's owner (retained `openPanel`/`updatePanel` `extensionId`, else the `<ext>/panel-<n>` prefix) as `oino_ui_action` in a **command** context, and returns once dispatched (handler errors become `extension_error`). Same for `invoke_shortcut` (shortcut context). Owners receive the event whether or not they subscribed. | Panel buttons replace sessions, open dialogs, run commands; awaiting them would block the frontend's command queue. | Frontends get no handler result. |
| 5 | ui-protocol §7.2 | Consumable intents (`emptyArrowDown`, `tokenBackspace`) wait for every subscribed handler until one returns `{consume: true}` (normal hook deadlines; the 50 ms budget is the frontend's). Other intents are dispatched in the background and answer `{consumed: false}`. `editor_event` also records the composer text for `ui.editor-text`. | — | — |
| 6 | modes §4 `_oino/service_call` | New command (not in the table): calls `handle-service` of `target` or the first extension that registered the service, caller `oino.frontend`, 30 s timeout. Core learns service names from `ExtensionInstance::services()` (WASM: the host hub's registrations). | GPUI widgets (unipi app-bridge R7), dynamic settings options. | — |
| 7 | modes §4 `_oino/login` | Sign-in runs the provider component's `oauth-login` in the session's call context (its `ui.*` dialogs reach the frontend), strips `type`, stores an `oauth` credential in `auth.json` and re-runs `list-models` for that provider with the credential (Copilot `availableModelIds`). `_oino/auth_providers` (new) lists providers for the picker. Providers without `oauth: true` answer "… does not support OAuth sign-in; set an API key instead". | modes §4 left the flow open. | The reply arrives after the whole flow; frontends send it outside an ordered queue. |
| 8 | runtime §4 | Core installs an `OAuthRefresher` that routes `auth.json` refreshes to the registering component's `oauth-refresh` (15 s). | Refresh was implemented in `oino-models` but never wired, so OAuth tokens expired. | — |
| 9 | extension-api §3 `models` | `models.stream` streams the provider events of one request (same model/credential/virtual routing as `complete`, no retries, `cacheRetention: none`) as `AssistantMessageEvent` JSON; `models.credential-for` returns `{provider, headers, baseUrl?}` with `Authorization: Bearer <resolved key>` plus configured headers, or `null`. New `HostApi::model_stream` and `HostOp::CredentialFor`. | Known issue 7. | `classify` and `generate-image` remain unsupported. |
| 10 | extension-api §3 `ui` | `ui.editor-text` returns the text from the last `editor_event`; `editor-set` / `editor-insert` fire `set_editor_text` / `insertEditorText` (extension id `oino.host`, the WIT call has no caller); `clipboard-read-image` and `edit-externally` are dialogs (`clipboardReadImage`, `editExternally{text, extensionHint?}`) whose `value` is returned. | Known issue 7. | Frontends that never send `editor_event` report `""` (pi RPC behaviour). |
| 11 | extension-api §6 | `ProviderInstance::oauth_login` (`oino-extension-api`) takes the `CallContext`; core's `ExtensionInstance` gained `complete_input`, `services`, `handle_service`; core's `ProviderInstance` gained `oauth_login` / `oauth_refresh` (all defaulted). | The WASM `oauth-login` ran with the init context (no session), so its dialogs had nowhere to go. | Native test doubles implement them optionally. |
| 12 | modes §6 / integration #9 | ACP sessions resolve `--model` / `--provider` / `--models` / `:thinking` per cwd through `SessionDefaults::model_resolver` (`oino_acp::ModelResolver`, `ModelChoice`), the same rules as `startup::choose_model` without an existing session; diagnostics go to stderr per session. | The registry exists only once the cwd's services do. | Known issue 5 closed. |
| 13 | modes §1.2 `oino update` | Bare `oino update [--check]` is the self-update; packages update with `oino update <id>…` or `oino update --extensions` (`--project` for `.oino/extensions`). | `update [id]` (packages) and `update [--check]` (self) collide in modes §1.2. | — |
| 14 | runtime §15 | Package manager lives in `crates/oino` (`cli::packages`), not `oino-core::packages`; URL/archive sources need the system `tar` (`--zstd` for `.tar.zst`), git sources the system `git`; downloads use `reqwest`. `.source.json` = `{source, version, installedAt}`. `config` prints the settings paths instead of opening an editor. Permissions are printed; non-interactive installs need `--yes`. | The CLI is the only user; no archive crates in the workspace. | `oino install` without `tar`/`git` works for directories only. |
| 15 | modes §1.2 `oino update` | Self-update reads `release-manifest.json` (latest, `OINO_REF`, or `OINO_RELEASE_MANIFEST`, `file://` allowed), picks the `oino` artifact of `current_target()`, verifies sha256 and renames the running binary to `.old` before moving the new one in (also on Windows). | No installer artifact on Windows releases yet. | — |

## TUI (`crates/oino-tui`, `crates/oino-view`)

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| T1 | tui-client #5/#6 | Every composer edit sends `_oino/get_completions`; backend groups other than `commands` (and `files` when a local popup is open) are merged into the popup and accepted with `Composer::replace_before_cursor(from, value)`. Local `/` and `@` completion stays (instant). | — | Extension argument and `complete-input` completions work. |
| T2 | tui-client #5 | Editor intents: `changed`/`cleared` debounced 150 ms (on ticks), `submit` before a prompt, `emptyArrowDown` on ↓ in an empty composer, `tokenBackspace` before deleting an attachment token (the deletion runs when the reply says nobody consumed it). | — | — |
| T3 | tui-client #7 | `/settings` lists the extension namespaces after the core rows; a namespace page (`oino_view::settings_hub`) renders sections/pages flattened, edits booleans/enums in place, text/number/secret/model fields in an inline input, runs `action` fields as `/command`, resets with Delete/ctrl+d, toggles the write scope with Tab (global-only/project-only fields ignore it). `multiselect`/`order` are shown read-only; no search, no advanced filter, no revert-on-open. `openSettings{namespace}` opens the page. | Scope of this pass. | Core settings still go through `TuiHost` (LocalHost). |
| T4 | tui-client #8 | `/login [provider]` lists OAuth providers (`_oino/auth_providers`) and runs `_oino/login` concurrently; `/logout [provider]` lists providers with stored credentials. | — | — |
| T5 | tui-client #2 | Panel actions and shortcuts go through the backend (`FrontendApi::ui_action` / `invoke_shortcut`); `TuiHost::ui_action` / `invoke_shortcut` were removed. | — | Works with `--remote` once it exists. |
| T6 | tui-client #3 | Regular (inline) layout: `--tui-mode regular` or setting `tuiMode: "regular"` (also a `/settings` row, next start). No alternate screen; an inline viewport (3/5 of the rows, ≥ 14) holds the live turn and the chrome; the header and finished rows are written above it (`Terminal::insert_before`) once the session is idle and never redrawn. | ratatui owns a frame; pi re-renders its components in place. | Rows of a running turn stay in the viewport until it settles; later changes to committed rows (labels, re-renders after resize) are not reflected in the scrollback. |
| T7 | tui-client #4 | Inline images are still `[image: <mime>]` placeholders (`images: false`). | Kitty/iTerm/sixel output needs per-terminal detection and an escape pass outside ratatui's buffer. | Open. |

## Requests for other work

- **gpui-desktop** (`apps/`): use `oino_client::FrontendApi` for panel actions
  (`session_pane.rs` logs "not wired"), shortcuts (`workspace.rs`), completions, the settings
  hub (`settings_schema` / `extension_settings` / `set_extension_setting`, model in
  `oino_view::settings_hub`), `/login` (`auth_providers` + `login`) and `service_call`
  (app-bridge widgets). `apps/Cargo.lock` needs a refresh (new deps: `oino-client` →
  `oino-settings`, `oino-core` → `ignore`, `oino` → `reqwest`/`sha2`/`tempfile`); it was left
  untouched here.
- **modes**: `--remote` / `oino serve` (a remote TUI now only lacks `TuiHost` core settings,
  session listing and keybindings over RPC: `_oino/list_sessions`, `_oino/get_keybindings`,
  `_oino/render`).
- **providers-v1**: providers whose `oauth-login` needs a loopback listener must keep working
  with the session call context (now passed); Copilot's `list-models` receives the stored OAuth
  credential (`{type: "oauth", access, refresh, expires, …}`) after `/login`.
- **core-runtime**: `classify` / `generate-image`; usage of `models.stream` is not recorded.
