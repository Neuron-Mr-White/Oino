# Extension UI protocol (frontend-agnostic)

Status: binding. Types: `oino_protocol::ui` (owner core-types-session). Producers: core-runtime
(UI bridge), extensions via `ui.*` WIT. Renderers: tui-client, gpui-desktop, gpui-mobile;
transports: modes (RPC/serve), acp.

## 1. Principles

1. **pi `rpc-extension-ui` records are kept verbatim** (methods, field names, response forms,
   timeout semantics). Oino adds methods and *optional* fields only. A pi RPC client that
   ignores unknown methods keeps working (D23).
2. Extensions emit **data**, never terminal output. Rich content uses the **Oino UI Spec
   (OUS) v1** node tree (§6). TUI and GPUI render every node natively; RPC clients get the
   same JSON.
3. One protocol in every transport: in-process (Rust enums over channels), RPC/serve (JSONL),
   ACP (mapped, modes §6).
4. **First answer wins** across attached frontends (D16); the core broadcasts a dismissal.
5. Frontends own keys, layout and theme; extensions name *intents* and *semantic tones*.

## 2. Message set

Core → frontend: `extension_ui_request` (dialogs and fire-and-forget) and
`extension_ui_dismiss` (Oino). Frontend → core: `extension_ui_response` (pi),
`extension_ui_action`, `editor_event`, `invoke_shortcut`, `get_completions`, `render`
(Oino; RPC command forms in modes §4).

```rust
// oino_protocol::ui
pub struct UiRequest { pub id: String, #[serde(flatten)] pub method: UiMethod,
    pub extension_id: Option<String>, pub session_id: Option<String> }   // Oino optional fields
#[serde(tag = "method")]
pub enum UiMethod { /* §4: select, confirm, input, editor, multiSelect, form, approval, custom,
    clipboardReadImage, editExternally, notify, setStatus, setWidget, setTitle, set_editor_text,
    setFrame, setWorkingMessage, setHeader, setFooter, openPanel, updatePanel, closePanel,
    insertEditorToken, osNotify, openUrl, clipboardWrite, setShortcuts, setTheme, openSettings */ }
pub enum UiResponse { Value{id, value: serde_json::Value}, Confirmed{id, confirmed: bool}, Cancelled{id} } // pi forms
```

`id`: UUID v4 string. Wire framing: `{"type":"extension_ui_request","id":…,"method":…,…}`.

## 3. Routing, lifetime, multi-frontend

- Dialog requests are delivered to **every attached frontend** of the session that declared
  `ui: true` in its client capabilities (modes §4 `_oino/hello`). The first valid
  `extension_ui_response` resolves the dialog; core then emits
  `{"type":"extension_ui_dismiss","id":…,"reason":"answered"|"timeout"|"cancelled"}` so other
  frontends close it. Responses for unknown/finished ids are ignored.
- Timeouts are agent-side (pi): on expiry the dialog resolves with its default (`select`/
  `input`/`editor` → `undefined`, `confirm` → `false`, Oino dialogs → cancelled) and a dismiss
  is broadcast. Extension cancel tokens cancel dialogs the same way.
- A dialog only has the timeout its request declares: extension handler deadlines pause while
  the handler's extension has a dialog open (GA-42). When the waiting handler is dropped or
  times out anyway, its dialogs are withdrawn with `extension_ui_dismiss{reason:"cancelled"}`
  (no orphan cards).
- No UI attached (`hasUI == false`: print/json modes, children with `UiPolicy::None`): dialogs
  resolve immediately as cancelled/default; fire-and-forget requests are dropped.
- `ui_prompt_start`/`ui_prompt_end` extension events wrap every dialog (runtime §7.2).
- Fire-and-forget state (status, widgets, frame, header/footer, title, working message,
  panels, shortcuts) is **retained by core per session** and replayed to a frontend when it
  attaches (`_oino/ui_snapshot`, modes §4), so late-joining frontends (phone) see current
  state. Keys are scoped per extension: the effective key is `<extensionId>/<key>`; pi-shaped
  records on the wire keep the extension's own key in `statusKey`/`widgetKey` plus the
  `extensionId` field.

## 4. Methods

### 4.1 pi dialogs (verbatim)

| method | Fields | Response |
|---|---|---|
| `select` | `title`, `options: string[]`, `timeout?` (ms) + Oino `optionDetails?: [{label, description?}]` (same order as `options`) | `value` (an option string) or `cancelled` |
| `confirm` | `title`, `message`, `timeout?` | `confirmed` or `cancelled` |
| `input` | `title`, `placeholder?`, `timeout?` + Oino `prefill?`, `secret?: bool` | `value` or `cancelled` |
| `editor` | `title`, `prefill?` + Oino `language?` | `value` or `cancelled` |

### 4.2 pi fire-and-forget (verbatim)

| method | Fields |
|---|---|
| `notify` | `message`, `notifyType?: info\|warning\|error` (default info) |
| `setStatus` | `statusKey`, `statusText?` (absent clears) + Oino `tone?` (§6.5), `priority?` (int, higher first) |
| `setWidget` | `widgetKey`, `widgetLines?: string[]` (absent clears), `widgetPlacement?: aboveEditor\|belowEditor` + Oino `widgetSpec?: OusNode` (preferred over lines when the frontend supports OUS), Oino placements `header\|footer` |
| `setTitle` | `title` |
| `set_editor_text` | `text` |

### 4.3 Oino dialogs

| method | Fields | Response (`value` shape) |
|---|---|---|
| `multiSelect` | `title`, `options: [{label, description?, value?, selected?}]`, `min?`, `max?`, `timeout?` | `value: string[]` (values or labels) |
| `form` | `title`, `questions: [FormQuestion]`, `submitLabel?`, `timeout?` | `value: FormAnswers` |
| `approval` | `title`, `toolCallId?`, `toolName?`, `summary`, `detail?: OusNode` (e.g. diff, command), `options: [{optionId, label, kind: allow_once\|allow_always\|reject_once\|reject_always}]`, `feedback?: {placeholder?}`, `timeout?` | `value: {optionId, feedback?}` |
| `custom` | `title?`, `spec: OusNode`, `actions?` (implicit from interactive nodes), `size?: small\|medium\|large\|full`, `timeout?` | `value: {actionId, value?}` (first submitting action) |
| `clipboardReadImage` | — | `value: ImageContent \| null` |
| `editExternally` | `text`, `extensionHint?` (e.g. `md`) | `value: string` or `cancelled` |

`FormQuestion = {id, question, header? (≤16 chars), options: [{label, description?, value?,
action?: "end_turn"|"new_session", prefill?}] (0–9), multiSelect?: bool, other?: bool
(free-text "Other"), attachments?: bool}`; `FormAnswers = {answers: [{id, selected: string[],
other?: string, attachments?: ImageContent[]}], action?: "end_turn"|"new_session"}`. This
covers unipi ask_user (unipi-port §3.13).

### 4.4 Oino fire-and-forget

| method | Fields | Notes |
|---|---|---|
| `setFrame` | `frameKey`, `top?: [Segment]`, `bottom?: [Segment]` (absent both = clear) | editor frame segments (footer package); `Segment = {text, tone?, icon?, priority (int), align: left\|right, minWidth?}` — frontends drop lowest priority first when narrow |
| `setWorkingMessage` | `message?`, `visible?` | pi `setWorkingMessage/Visible` |
| `setHeader` / `setFooter` | `key`, `spec?: OusNode` | startup splash/header and footer area; absent clears |
| `openPanel` / `updatePanel` / `closePanel` | §5 | |
| `insertEditorToken` | `token: {tokenId, kind: image\|file\|text, label, data?: ImageContent, path?}` | attachment token in composer (§7.2) |
| `osNotify` | `title`, `body`, `urgency: low\|normal\|critical`, `actionUrl?` | frontend uses OS notification; TUI → OSC 777/9 or `notify-send` via host |
| `openUrl` | `url` | |
| `clipboardWrite` | `text` | TUI → OSC 52 |
| `setShortcuts` | `shortcuts: [{id, keys, description, extensionId}]` | full current list (on change); emitted by core from the shortcut registry after `session_start` and on every (un)registration, retained |
| `setTheme` | `name` | pi `setTheme`; frontends without themes ignore |
| `openSettings` | `namespace?`, `query?` | opens the frontend settings hub (§6.6) filtered to a namespace/search (unipi `/settings`, `/skills`) |

## 5. Panels (views)

```json
{"type":"extension_ui_request","id":"…","method":"openPanel","panelId":"unipi.info-screen/dashboard",
 "placement":"overlay","title":"Info","spec":{…OusNode…},"keys":[{"key":"r","actionId":"refresh","label":"Refresh"}],
 "focus":true,"closeOnEscape":true}
```

| Field | Values |
|---|---|
| `placement` | `overlay` (modal over transcript), `sidePane` (GPUI right pane tab; TUI = overlay), `editorReplace` (replaces the composer: work tray, btw panel; Esc returns), `bottomPane` (GPUI bottom row; TUI = editorReplace), `fullscreen` |
| `keys` | panel-level key hints → actions (TUI binds keys; GPUI shows a toolbar/menu and binds the same keys when focused) |
| `focus` | take focus on open |

`updatePanel{panelId, spec?, title?, keys?}` replaces the given parts (full spec replacement;
frontends diff by node `key`). `closePanel{panelId}`. Frontend → core
`extension_ui_action{panelId, actionId, value?}` delivers an `oino_ui_action` event to the
owning extension; `closed` is sent as `actionId: "$close"` when the user closes a panel.
Panel ids are unique per session (`<extensionId>/<name>`); opening an existing id replaces it.
Max spec size 256 KiB (larger ⇒ request rejected with an `extension_error`).

## 6. OUS v1 — node vocabulary

### 6.1 Node shape

Every node: `{"t": "<kind>", "key"?: string, ...props}`. Unknown `t` ⇒ frontend renders
`fallback` prop if present, else nothing. Text props accept `Rich = string | Span[]`;
`Span = {text, tone?, bold?, italic?, dim?, underline?, code?, link?: url}`.

| `t` | Props | TUI rendering | GPUI rendering (gpui-kit) |
|---|---|---|---|
| `text` | `value: Rich`, `wrap?: bool` (default true) | paragraph | `Label`/`StyledText` |
| `markdown` | `value: string` | pulldown-cmark renderer | `TextView` markdown |
| `code` | `value`, `language?`, `lineNumbers?` | syntect | `TextView` code / `Input` read-only |
| `diff` | `patch` (unified) or `{oldText, newText, path?}` | colored unified diff | `diff` component |
| `image` | `data` (base64) + `mimeType`, or `path`; `alt`, `maxWidth?` | kitty/iterm/sixel if supported else `alt` | `img` |
| `log` | `lines: string[]`, `tail?: bool`, `maxLines?` | last N lines | virtual list, monospace |
| `kv` | `items: [{key: Rich, value: Rich}]` | two columns | `description list` |
| `table` | `columns: [{title, align?, width?: auto\|fill\|N}]`, `rows: [[Rich]]`, `selectable?: actionId` | ratatui `Table` | `table` |
| `list` | `items: [{id, label: Rich, detail?: Rich, badge?: Rich, tone?}]`, `selectable?: actionId`, `selected?: id`, `multi?: bool` | list with cursor | `list` |
| `tree` | `nodes: [{id, label: Rich, children?: […], expanded?}]`, `selectable?: actionId` | indented tree | `tree` |
| `tabs` | `tabs: [{id, title, body: Node}]`, `active?: id`, `onChange?: actionId` | tab bar | `tab` |
| `split` | `direction: horizontal\|vertical`, `children: [Node]`, `ratios?: number[]` | layout | `resizable` |
| `stack` | `direction`, `gap?`, `children` | layout | flex div |
| `section` | `title: Rich`, `body: Node`, `collapsible?`, `collapsed?` | titled block | collapsible |
| `card` | `title?: Rich`, `rail?: tone`, `source?: string` (provenance label), `body: Node` | bordered block, colored rail | `bubble`/card |
| `badge` | `value: Rich`, `tone?` | `[x]` styled | `tag` |
| `meter` | `value: number`, `max: number`, `label?`, `tone?` | bar | progress |
| `gauge` | `value 0..1`, `label?` | gauge | circular/linear progress |
| `sparkline` | `values: number[]`, `label?` | braille sparkline | chart line |
| `barChart` | `bars: [{label, value, tone?}]` | bars | chart bars |
| `area` | `series: [{label, values: number[]}]` | braille area | chart area |
| `spinner` | `label?` | animated (frontend ticker) | `spinner` |
| `divider` | `label?` | rule | separator |
| `keyHints` | `hints: [{keys: string, label}]` | dim hint line | `kbd` row |
| `input` | `id`, `value?`, `placeholder?`, `onSubmit: actionId`, `multiline?` | text field | `Input`/`Textarea` |
| `toggle` | `id`, `label`, `value: bool`, `onChange: actionId` | `[x]` | `switch` |
| `select` | `id`, `options: [{value, label}]`, `value?`, `onChange: actionId` | cycling field | `select` |
| `button` | `label`, `actionId`, `tone?`, `primary?` | `[ Label ]` | `button` |
| `empty` | `message?` | dim text | empty state |

Interactive nodes send `extension_ui_action{panelId, actionId, value}`; `value` is the item id,
input text, bool or selected value. For `custom` dialogs the first `button`/`onSubmit` action
resolves the dialog.

### 6.2 Limits

Depth ≤ 32, nodes ≤ 10 000, `log.lines` ≤ 5 000, image ≤ 4 MiB decoded. Frontends may
virtualise lists/tables/logs.

### 6.3 Animation and time

Spinners and relative times are rendered by frontends (one shared ticker); extensions never
push frames for animation (unipi shared ticker → frontend).

### 6.4 Transcript render specs

Custom messages, custom entries and extension tools may have renderers (pi
`registerMessageRenderer`/`registerEntryRenderer`/`registerToolRenderer` as data):

- Registration: `registry.register-renderer({target, key})` (extension-api §3).
- Frontend asks lazily: `render{target: customMessage|customEntry|tool, key, entryId?,
  density: simple|regular|advanced, expanded: bool, width: narrow|normal|wide, data}` where
  `data` = the message / entry / `{toolCall, args, result?, partialResult?, isError?}`. Core
  calls the guest `render` export (synchronous, 50 ms deadline) and returns `OusNode | null`.
  `null` or timeout ⇒ default rendering. Results are cached by `(entryId|toolCallId, density,
  expanded, width, contentHash)`.
- `ToolRenderHints` on `tool-spec.render` (no guest call): `{title?: template string with
  {arg} placeholders, icon?: string, collapsedSummary?: template, showArgs?: string[]}`.
- Built-in tools (read/write/edit/bash/grep/find/ls) are rendered natively by frontends using
  their typed details (diff for edit, tail for bash). A render spec for a built-in name is a
  **render-only override**: core accepts it when the manifest's `tools.override` lists the tool
  (or `"*"`), drops it otherwise, and dispatches `render` for built-in rows too (U1–U2 landed
  2026-10-29; `unipi.utility`'s `render.style` simple/regular/advanced rows). Frontends that
  ask: the TUI for every tool row when its host has a session (`_oino/render`,
  frontend-api §13); the GPUI app still renders built-in rows itself.
- Provenance (`details.oinoProvenance`, runtime §2) renders as a `card` with `source` label and
  `rail` tone `harness` regardless of renderer.

### 6.5 Tones and theme tokens

`tone ∈ default | muted | accent | info | success | warning | error | harness | thinking |
tool | user | assistant | c1 … c12` (`c*` = categorical palette for package colors). Frontends
map tones to their theme; no hex colors in OUS. Icons: a small name set (`check`, `x`, `dot`,
`warn`, `info`, `clock`, `gear`, `branch`, `bolt`, `bot`, `folder`, `file`, `search`, `cloud`,
`lock`) — frontends map to glyphs (TUI Unicode/Nerd Font per setting, GPUI icon set).

### 6.6 Settings schema (for the settings hub)

```json
{ "namespace": "notify", "title": "Notify", "extensionId": "unipi.notify",
  "sections": [ { "id": "events", "title": "Events", "fields": [
    { "key": "events.agentEnd", "type": "boolean", "label": "Agent finished", "default": true, "scope": "both" },
    { "key": "platform", "type": "enum", "label": "Platform", "options": [{"value":"native","label":"Native","description":"OS notifications"}] },
    { "key": "ntfy.token", "type": "secret", "label": "ntfy token", "scope": "global" },
    { "key": "order", "type": "order", "label": "Order", "items": ["a","b"] },
    { "key": "model", "type": "model", "label": "Model", "filter": {"input": ["image"]} },
    { "key": "test", "type": "action", "label": "Send test", "command": "notify-test" },
    { "key": "advanced", "type": "page", "label": "Advanced", "sections": [ … ] }
  ] } ] }
```

Field types: `boolean`, `enum`, `multiselect`, `string` (`multiline?`, `pattern?`), `number`
(`min?`, `max?`, `step?`, `unit?`), `secret`, `order`, `model`, `action` (runs
`command-invoke`), `page`. Common props: `label`, `description?`, `hint?`, `badge?`,
`default?`, `scope: both|global|project` (default both), `advanced?: bool`, `dynamic?:
{service: name}` (options fetched via `services.call`). Frontends render one hub over all
namespaces with search, scope toggle (global/project), edit, reset-to-default, revert-on-open
(unipi hub semantics). Commands: `get_settings_schema`, `get_extension_settings{namespace,
scope}`, `set_extension_setting{namespace, key, value, scope}`, `unset_extension_setting`
(modes §4).

## 7. Editor model

### 7.1 Editor operations

`ui.editor-text/editor-set/editor-insert` (extension → core → frontend via
`set_editor_text` / `insertEditorText{text}`). The frontend owns the composer; core keeps the
last reported text (`editor_event{intent:"changed"}` debounced 150 ms) so `editor-text` works
(pi RPC returns `""`; Oino frontends report text).

### 7.2 Intents (`editor_event`, frontend → core → `oino_editor` to subscribed extensions)

| intent | When | Consumable |
|---|---|---|
| `changed` | text changed (debounced) | no |
| `cleared` | text became empty | no |
| `emptyArrowDown` | ↓ pressed while composer empty | yes (frontend waits ≤ 50 ms for a `consume` reply; else default behaviour) |
| `submit` | before sending (after `input` event semantics are irrelevant here) | no |
| `tokenBackspace` | Backspace adjacent to an attachment token (`tokenId`) | yes |

Consumption reply: `editor_event_reply{eventId, consumed: bool}` is produced by core from the
first subscribed extension whose `on-event` result is `{consume: true}`.
Attachment tokens: `insertEditorToken` places an atomic token in the composer; on submit the
frontend sends images as `images` and file tokens as `@path` text (core expands `@path` per pi
CLI semantics only in print mode; in interactive prompts the text is sent as-is and extensions
resolve tokens).

### 7.3 Autocomplete

Frontend → core `get_completions{text, cursor}` → core returns merged
`[CompletionGroup{source, items: [{value, label?, description?, tag?, tone?, sortTier?}]}]`
from: slash commands (`/name`, with `complete-command` for args), skills (`/skill:`), prompt
templates, `@` file paths (core file index, respects .gitignore), and extension
`complete-input` exports (unipi command-enchantment), each with a 100 ms budget.

## 8. Shortcuts

- Key ids use pi `KeyId` syntax (`ctrl+shift+b`, `alt+h`, `shift+down`, `f5`).
- Extensions register **action ids** with default keys; frontends resolve conflicts against
  their own reserved bindings and the user `keybindings.json` (`extensionShortcuts: {"<id>":
  ["alt+h"]}`), then invoke `invoke_shortcut{id}` → guest `run-shortcut` with a command-capable
  context.
- GPUI lists every shortcut in the command palette (invocable without keys); mobile shows them
  in a "Actions" sheet.

## 9. pi `ExtensionUIContext` → protocol

| pi method | Oino |
|---|---|
| `select`, `confirm`, `input`, `editor` | §4.1 |
| `notify` | `notify` |
| `setStatus`, `setWidget` (string arrays), `setTitle`, `setEditorText` | §4.2 |
| `setWidget` with component factory | `widgetSpec` (OUS) |
| `getEditorText`, `pasteToEditor` | `editor-text`, `editor-insert` |
| `onTerminalInput` | not available; use editor intents (§7.2) and shortcuts (§8) |
| `setWorkingMessage/Visible/Indicator`, `setHiddenThinkingLabel` | `setWorkingMessage`; indicator/label are frontend settings |
| `setFooter`, `setHeader` | `setFooter`, `setHeader` (OUS) |
| `custom` | `custom` dialog or `openPanel` |
| `addAutocompleteProvider` | guest `complete-input` |
| `setEditorComponent` / `getEditorComponent` | not available (frontend-owned); editor-replacing views use `openPanel{placement:"editorReplace"}` |
| `theme`, `getAllThemes`, `getTheme`, `setTheme` | `setTheme`; theme list via `get_themes` (modes §4) |
| `getToolsExpanded`, `setToolsExpanded` | frontend state; `ui.density` setting |

## 10. Frontend capability declaration and degradation

Frontends declare in `_oino/hello` (or in-process `UiCapabilities`): `{dialogs: [methods],
ous: bool, panels: [placements], images: bool, osNotify: bool, clipboard: bool}`. Core
degrades per frontend:

- No OUS ⇒ `widgetSpec`/panels rendered to plain lines (`OusNode::to_lines(width=80)` in
  `oino_protocol::ui`) and sent as `widgetLines` / a `custom` dialog becomes `select` over its
  buttons; `form` becomes sequential `select`/`input`; `multiSelect` becomes repeated `select`
  with a "Done" option; `approval` becomes `select` over option labels.
- Unsupported fire-and-forget ⇒ dropped.
- Pi RPC clients (no `_oino/hello`) get only pi methods (degraded as above).

## 11. Acceptance

- `oino_protocol::ui` round-trips every pi example in `rpc-extension-ui.md` byte-for-byte
  modulo whitespace; degradation functions unit-tested.
- `ui-probe` fixture passes against TUI (headless test backend), GPUI (view tests) and RPC.
- First-answer-wins tested with two in-process frontends attached to one session.

---

## Appendix: settled deviations (folded at 1.0.0-alpha.2)

Collected from the deviation files of the producers (unipi-*) and consumers (modes, acp, frontend-cmds).

Oino additions that bundled components emit and the TUI / desktop / mobile frontends consume.
Frontends that do not know a field ignore it; unknown OUS nodes render their `fallback` (§6.1).

| # | Where | Addition | Source |
|---|---|---|---|
| U1 | §6.1 OUS | `responsive {variants: [{minWidth, node}], fallback}` ✅ implemented: the first variant whose `minWidth` ≤ available width renders; none ⇒ `fallback`. | unipi-core #4, R4 |
| U2 | §7.3 completions | `CompletionGroup.from` (character offset the accepted item replaces), `replaces: "commands"` (contributor supersedes the frontend's slash group), `prefix`; items carry `tag`, `tone` (`c1…c12`), `sortTier`. | frontend-cmds #2, unipi-ux #1 |
| U3 | §7.2 editor intents | `cleared` may carry `reason: "interrupt" \| "submit" \| "edit"`; consumable intents (`emptyArrowDown`, `tokenBackspace`) wait for the first `{consume: true}`; `ui.editor-text` returns the text of the last `editor_event`. | unipi-ux #3, frontend-cmds #5, #10 |
| U4 | §4.3 dialogs | `form` hints `escape`, `digitAdvance`, `helpLine`, `clarify`; answers `attachments` / `files`. `custom` dialogs: `keys: [{key, actionId, label}]`, `closeOnUnknownKey`. `editor`: `cursorLine`. New dialogs `clipboardReadImage`, `editExternally{text, extensionHint?}`. | unipi-ux #4, #5; unipi-utility #5; frontend-cmds #10 |
| U5 | §6.3 frame / footer | `setFrame` segments: `id`, `priority`, `bold`, `style: "badge"`, `effect: "rainbow"`, `spans` + `fallback`; request-level `effect`, `iconStyle`, `colorMode`; `setFooter{key, spec, segments, separator, minRows}`. | unipi-ux #6 |
| U6 | §6 OUS | `barChart.columns`, `list.columns: "auto"`, span `strike`, `card.effect`, `meter.effect` / `durationMs`, `code{language: "mermaid"}` (frontends may render diagrams), header `effect` hints (`shimmer`, `drain`). | unipi-ux #8, #11; unipi-agents #20 |
| U7 | §5 fire-and-forget | `setPlan {entries: [{content, priority, status}]}` (ACP `plan` update; other frontends may show it in the status panel). | acp #10 |
| U8 | §6.1 panels | Panel ids default to `<ext>/panel-<n>`; `ui_action` routes to the panel owner as `oino_ui_action` in a command context and returns once dispatched. | integration #7, frontend-cmds #4 |
| U9 | §10 | Clients without `_oino/hello`: Oino-only dialogs resolve cancelled, Oino fire-and-forget methods are dropped, `extensionId` / `sessionId` / `extension_error.stack` are stripped (no degradation renderers). | modes #10 |
