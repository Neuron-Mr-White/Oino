# GUI automation driver (`[gui-driver]`)

Status: working (2026-10-29). Closes the two open harness gaps of
[STATUS.md §1b.7](STATUS.md#1b7-open-harness-gaps-round-3) on `ti`/Xvfb:

1. **Pointer injection** — clicks go through `Window::dispatch_event`, GPUI's own input entry
   point, so hit testing, focus transfer, click synthesis and pointer-only surfaces (board FAB,
   `+` launcher, settings switches, browser take-over, mascot toggle) behave exactly as they do
   for a person at the machine.
2. **Screen reading** — a semantic element tree (id, role, label, state, bounds) replaces OCR over
   screenshots for assertions.

Related: [ti.md](ti.md) (the GUI host), [gui-qa.md](gui-qa.md) (the QA round that hit the gaps).

## 1. Shape

```
apps/oino-ui/src/automation/
  registry.rs   per-frame element registry + the `probe` element wrapper (`Role`, `Meta`, `Node`)
  protocol.rs   JSON-lines request/response types, selectors
  server.rs     Unix-socket server (plain thread) → UI thread channel
  driver.rs     UI-thread request loop: tree/state/click/type/key/scroll/focus/wait_for/screenshot/quit
  input.rs      platform events (MouseDown/Up/Move, Scroll, Keystroke) built for `dispatch_event`
  state.rs      screen/session/dialog snapshot + the focus handles seen during a frame
scripts/gui-drive.py           client: commands, remote SSH pipe, scenario runner
docs/dev/v1/gui/scenarios/*.json   example scenarios
```

The layer is compiled in by default and **inert** until the app is started with
`OINO_DESKTOP_AUTOMATION=<unix socket path>`:

```bash
cargo xtask ti start --demo --env OINO_DESKTOP_AUTOMATION=/home/ti/oino-test/run/automation.sock
scripts/gui-drive.py --ti tree --role switch
```

### Zero cost when off

`registry::enabled()` is a relaxed atomic load; every probed element checks it once per
`request_layout` and once per `prepaint` and returns immediately when automation is off (the
same check gates the end-marker). No socket, thread, timer or task exists unless
`OINO_DESKTOP_AUTOMATION` is set — that also means every scenario step is opt-in, never part of
a normal run.

### Frame hooks

GPUI renders → lays out → prepaints → paints. Components wrap their element with
`automation::probe(el, meta)` (or `.automation(meta)` at the end of a `div()` chain), which
hooks two of those phases:

* **`request_layout`** — the element exists this frame, with this `Meta` (role, label, state,
  focus). GPUI runs it for every element of every view, including views it renders through a
  cached element tree, so it is the liveness signal.
* **`prepaint`** — where the element is (bounds, visibility). GPUI *reuses* the prepaint of a
  cached child view while its bounds, content mask and text style are unchanged, which is safe
  here: the previous geometry still describes the element.

`begin_frame(cx)` (top of `AppShell::render`) publishes the focus handles seen last frame, and
a zero-size paint end-marker (`automation::end_marker()`, after `perf::end_marker()`) publishes
the snapshot once the whole frame is prepainted — a frame that changes the UI is often the last
one before the app goes idle, so publishing at paint time (not on the next render) is what makes
`tree` current.

Two consequences worth knowing: the visible session is rendered **uncached** while automation
is on (`views/shell.rs`), because a cached view only re-renders when it is notified; and the
tree costs one hash-map insert per element per frame.

## 2. RPC

One JSON object per line, in both directions. Requests are handled strictly in order, so a
scenario is deterministic. Every mutating request lets two frames pass before the next one is
handled.

```json
{"id": 1, "method": "click", "params": {"id": "ap-allow"}}
{"id": 1, "ok": true, "result": {"at": [1210.5, 640.0], "id": "ap-allow", "handled": true}}
```

| Method | Params | Result |
|---|---|---|
| `ping` | — | `{pong, version}` |
| `tree` | optional selector (`id`, `role`, `label`), `include_hidden` | `{elements[], frame, window{width,height}}` |
| `state` | — | `{route, session_id, session_name, dialogs[{id,method,title}], panes, busy, open[], frame}` |
| `click` | selector **or** `x`,`y` | `{at, id, handled}` |
| `double_click` | selector or `x`,`y` | same |
| `right_click` | selector or `x`,`y` | same |
| `hover` | selector or `x`,`y` | same (a `MouseMove` only) |
| `type_text` | `text`, `submit` | `{typed, chars, submit}` |
| `key` | `key` (`enter`, `tab`, `secondary-k`, `shift-secondary-p`, …) | `{key, handled}` |
| `scroll` | selector or `x`,`y`, `dy`, `dx` (lines) | `{at, dx, dy, id}` |
| `focus` | selector (inputs register a focus handle) | `{focused}` |
| `wait_for` | selector and/or `text`, `absent`, `timeout_ms` | `{matched, ms, frame}` |
| `screenshot` | `path` (on the app host) | `{path, width, height}` |
| `settle` | — | `{frame}` |
| `drop` | selector or `x`,`y`, `paths` (on the app host) | `{at, id, paths}`: files dragged onto the window |
| `clipboard_image` | `path` of a PNG on the app host | `{path, bytes}`: puts it on the clipboard for `secondary-v` |
| `quit` | — | `{quitting}` |

**Selectors.** `id` is the element's own `ElementId` (`send`, `ap-allow`, `card-OIN-12`);
`role` is one of `button`, `icon_button`, `menu_row`, `tab`, `switch`, `checkbox`, `input`,
`row`, `card`, `option`, `segment`, `chip`, `text`, `other`; `label` is a case-insensitive
substring of the label or the value. Fields combine (AND). Resolution prefers visible elements,
then an exact id; an ambiguous selector fails with the candidate list
(`2 elements match …: ap-deny (button, "Reject"), ap-never (button, "Always reject")`).

**`text`** (in `tree` labels, `wait_for` and assertions) is the element label or value, which
for transcript rows is the row text (capped at 400 characters) and for toasts the toast message.

**`secondary-`** resolves from the host OS (`cmd` on macOS, `ctrl` elsewhere), so one scenario
file works on every platform.

**`screenshot`** shells out to ImageMagick `import` (`xdotool search --onlyvisible --class
dev.oino.desktop` for the window id, `root` as a fallback) and writes the PNG **on the app host**;
the client fetches it with `ssh … cat` when it is remote.

**Elements registered** — buttons (`Btn`, icon buttons), menu rows, tab pills, underline tabs,
switches, segmented options, dialog option rows (approval / question / dock / plan), session
list rows, nav items (Sessions / Board / Automations / Extensions / Settings), the palette rows
and filter, the composer, the send / attach buttons, the board FAB, board cards, lane pills and
"Add task" rows, toast text, transcript rows (text), settings nav rows, `+` launchers.

## 3. Injection is real input

`click` = `MouseMove` → `MouseDown` → `MouseUp` at the element's centre, each handed to
`Window::dispatch_event` — the same call the X11 / Wayland / macOS / Windows backends make. That
means GPUI's hit test picks the target from the *rendered frame*, hover and press states update,
a `track_focus` element takes keyboard focus, the click is synthesized by GPUI's hitbox
machinery (`is_hovered` + pending down), and `on_mouse_down(Right, …)` fires for `right_click`.
`type_text` produces one `Keystroke` per character (`key`, `key_char`, `shift` for capitals) and
dispatches them with `dispatch_keystroke`, so keymaps, the composer's `capture_key_down`, IME and
the platform input handler all run.

No view method is called directly: if a surface is unreachable this way, it is unreachable for a
person too.

## 4. Client and scenarios

```bash
# one-shot commands (local socket, or `--ti`/`--host`/`--socket` for a remote app)
scripts/gui-drive.py --ti tree                       # print the element list
scripts/gui-drive.py --ti tree --role switch          # filter
scripts/gui-drive.py --ti click --label "Allow once"
scripts/gui-drive.py --ti type "hello" --submit
scripts/gui-drive.py --ti key secondary-k
scripts/gui-drive.py --ti wait --text "Done" --timeout 30000
scripts/gui-drive.py --ti shot docs/dev/v1/design/shots/gui/x.png

# a scenario: restarts the app on ti with the scenario's demo scene, runs the steps,
# fetches screenshots into docs/dev/v1/design/shots/gui/<name>/
scripts/gui-drive.py --ti run docs/dev/v1/gui/scenarios/approval-allow.json
```

Scenario format (JSON):

```json
{
  "name": "approval-allow",
  "scene": "approval",
  "shots": "docs/dev/v1/design/shots/gui",
  "steps": [
    {"wait_for": {"id": "ap-allow", "timeout_ms": 60000}},
    {"assert": {"dialogs": "approval", "busy": true}},
    {"screenshot": {"name": "01-card"}},
    {"click": {"id": "ap-allow"}},
    {"wait_for": {"id": "ap-allow", "absent": true, "timeout_ms": 20000}},
    {"assert": {"dialogs": 0}},
    {"screenshot": {"name": "02-done"}}
  ]
}
```

Scenarios also choose the app backend and extra environment:

```json
{"name": "rewind-real", "backend": "real", "env": ["OINO_DESKTOP_DEMO_PACE=8"], "steps": []}
```

`backend` is `demo` (default, scripted scene), `faux` (real core, no network) or `real`
(providers from the copied `auth.json`; the client writes `settings.json`
`{"model": "oino:deepseek/deepseek-flash"}` and an onboarded `desktop.json`, and copies a
solid-red test PNG to `~/oino-test/run/attach-red.png` for the attachment scenarios).
`--backend` overrides it. `env` entries go to `cargo xtask ti start --env`.

Steps: any RPC method (`click`, `double_click`, `right_click`, `hover`, `type_text`, `key`,
`scroll`, `focus`, `wait_for`, `tree`, `state`, `screenshot`, `settle`, `drop`,
`clipboard_image`, `ping`) plus `sleep`
(`{"ms": 250}`), `assert` and `wait_state` (retries `assert` until it holds or `timeout_ms`
expires), and `toggle` (clicks a switch and proves its `checked` state flipped, whatever the
persisted value was). `assert`/`wait_state` accept `route`, `busy`, `dialogs` (count or method),
`session_id`, `text`, `element` (`{"id", "role", "label", "checked", …}`), `absent`. A failed
step takes a `FAIL-<n>.png` screenshot and prints `state`; the run continues unless
`--stop-on-failure` was passed. `--no-restart` reuses a running app, `--scene` overrides the
scenario's demo scene.

`scene` is the `OINO_DESKTOP_DEMO_SCENE` of the `--demo` backend (`welcome`, `run`, `approval`,
`question`, `plan`, `board`, `info`, `stream`), so scenarios are deterministic and free
(`apps/oino-desktop/src/demo.rs`). With a real backend, drop `scene` and the same steps apply.

### Shipped scenarios

| File | Covers |
|---|---|
| `new-session-prompt.json` | click the composer, type a prompt, Enter, the assistant plan streams |
| `approval-allow.json` | the docked approval card: allow, the bash tool runs, the turn finishes |
| `approval-deny.json` | the docked approval card: reject, the tool is blocked, the turn still finishes |
| `ask-user-form.json` | the `ask_user` form: pick an option per step, Next, Submit |
| `side-pane-tabs.json` | `hdr-side`, the `+` launcher menu, opening Files and Tree tabs, switching tabs |
| `board-task.json` | nav → Board, the FAB dialog (create), card select, the task detail's move button |
| `settings-switches.json` | settings nav → Unicrab, `toggle` of the mascot and native-Wayland switches |
| `agents-live.json` | AgentRow live status, the Agent tab's read-only child timeline (`_oino/agents` + `agent_messages`), Stop agent, and the `btw` child opened from the StatusPanel's Run row |
| `status-panel.json` | the StatusPanel fed by the sticky topics (Goal `turn 2/5`, tasks, sidekick, kanboard claims, git), header click → focus, ↓ + Enter → child Agent tab, Delete → stop confirm, ⌘⇧S collapse/expand |
| `turn-undo.json` | the turn footer's Undo: confirmation, files restored toast, checkpoint still offered |
| `queue-edit.json` | two queued follow-ups, ↑ reorder, recall to the composer, remove, empty queue |
| `image-attach.json` (real) | a dropped PNG becomes `Image #1` and the prompt carries it (the model answers the colour) |
| `subagent-real.json` (real) | a real `run_subagent` call → AgentRow → Agent tab |
| `rewind-real.json` (real) | user bubble Rewind on a short transcript: confirm → fork → composer refilled |

## 5. `ti` specifics

`cargo xtask ti start` gained a repeatable `--env KEY=VALUE` (used for the socket and the demo
scene). The client restarts the app itself in `run` mode:

1. `cargo xtask ti stop`, then `cargo xtask ti start --demo --env OINO_DESKTOP_AUTOMATION=… --env
   OINO_DESKTOP_DEMO_SCENE=<scene>` (see [ti.md](ti.md)).
2. `ssh ti@ti python3 -u -c '<proxy>' <socket>` — the socket is bridged over SSH, so no extra
   port and no `socat` is needed.
3. Screenshots land under `~/oino-test/run` (or the scenario's `--remote-path-prefix`) on `ti`
   and are fetched back into `docs/dev/v1/design/shots/gui/<scenario>/`.

`scripts/gui-drive.py --ti quit` (or `cargo xtask ti stop`) leaves nothing running on the host.

## 6. Tests and gates

* `apps/oino-ui/src/automation/*` unit tests (12): protocol parse/reject, selector matching, the
  keystroke mapping (`secondary-` per OS, one keystroke per character), the state JSON, the
  snapshot rules (duplicate ids get `#n`, geometry survives a reused prepaint, elements that
  stop being reported disappear), and an end-to-end socket test (two request/response round
  trips plus a bad-JSON line that keeps the connection alive).
* Gates for the touched apps: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
  -- -D warnings -W clippy::pedantic`, `cargo test --workspace` (in `apps/`), and `cargo clippy -p
  xtask …` for the `--env` flag.

```bash
cd apps && TMPDIR=$PWD/../target/tmp cargo test -j 4 -p oino-ui --lib automation
```

## 7. Proof on `ti` (`a0b75e6`…`bd39807`, 2026-10-29)

Every run below is `scripts/gui-drive.py --ti run <scenario>` on `ti` (`Xvfb :93`, lavapipe,
release build of `bbebb63`+). Each scenario restarts the app itself, so nothing else was
running and no state leaked between them. All seven PASSED in one sequential run:

| Scenario | Steps | Result |
|---|---|---|
| `new-session-prompt` | 9 | **PASS** — click composer, type 38 chars, Enter, plan streams |
| `approval-allow` | 12 | **PASS** — allow, tool runs, fold expands to `Compiling json-lite` / `14 passed` |
| `approval-deny` | 9 | **PASS** — reject, fold shows `Blocked by unipi.workflow: denied by user` |
| `ask-user-form` | 12 | **PASS** — step 1 option, Next, step 2 multi-select, Submit |
| `side-pane-tabs` | 15 | **PASS** — `hdr-side`, `+` launcher, Files + Tree tabs, tab switch |
| `board-task` | 19 | **PASS** — nav → Board, FAB dialog create, card select, `→ In Progress` move |
| `settings-switches` | 14 | **PASS** — `set-mascot` false→true→false, `set-wayland` false→true |

Screenshots: `docs/dev/v1/design/shots/gui/<scenario>/*.png` (fetched from `ti`).

Pointer-only surfaces from §1b.7, each driven with the mouse only (no shortcut, no config edit):

| Surface | Driver evidence |
|---|---|
| board FAB (`board-fab`) | `board-task` steps 6–11: click opens the New task dialog, typed title, `Add task` closes it |
| `+` launcher (`side-add`) | `side-pane-tabs` steps 4–12: click opens the menu, `Files` / `Tree` rows open tabs |
| settings switches | `settings-switches`: `set-mascot` flips `checked` in `tree` (false→true→false), `set-wayland` too |
| mascot toggle | same switch (`set-mascot`) — it is what opens/closes the mascot window |
| browser take-over (`br-takeover`) | **partially**: verified with `--faux` that the browser tab, its tab row and the toolbar (`br-new`, `br-back/fwd/reload/ext`) are pointer-drivable and report `enabled`; the take-over strip itself only renders while an agent drives a tab (`Strip::AgentDriving`/`UserControl`), which the demo/faux backends never do. The strip's button is a `Btn`, the same element and dispatch path proven by `ap-allow`, `board-move` and `q-next`. |

Two findings from the proof round, both fixed in the layer:

1. A cached child view (`Entity::cached`) is only re-rendered when it is notified, and GPUI
   reuses its prepaint while bounds are unchanged — so the registry publishes existence from
   `request_layout` (always runs) plus geometry from `prepaint`, and the shell leaves the
   visible session uncached while automation is on (`views/shell.rs`, `automation::enabled()`).
2. A frame that changes the UI is often the last one before the app goes idle, so the snapshot
   is published from a zero-size paint end-marker, not from the next render.

## 7b. Proof of the gui-wiring scenarios (`6ef6f83`…`be6ce69`, 2026-10-29)

Release build of `be6ce69`, `Xvfb :93`, lavapipe, one app instance per scenario (each
`run` restarts the app itself):

| Scenario | Backend | Steps | Result |
|---|---|---|---|
| `agents-live` | demo | 20 | **PASS** — AgentRow live (`run_subagent`, 1 step, 20.9 s), Agent tab timeline ("Map the parser's call sites…"), Stop agent → `Agent session · done`, StatusPanel Run row `btw` → its child timeline |
| `status-panel` | demo | 31 | **PASS** — mini capsule `Status · 0/0 · 5 running · 3 changed`, expand, header focus, ↓ + Enter → `btw` Agent tab, Delete → `Stop agent` confirm → row gone, sections `Goal · turn 2/5`, `cargo test --watch 4m12s`, `Haiku`, `Kanboard · 1 claimed`, `Git · v1-alpha · 3 changed`, ⌘⇧S collapse/expand |
| `turn-undo` | demo | 13 | **PASS** — Undo → confirm → `Restored 1 file`, checkpoint stays |
| `queue-edit` | demo | 19 | **PASS** — two follow-ups queued, ↑ reorder, recall refills the composer, remove empties the queue |
| `image-attach` | real | 12 | **blocked** — the drop lands and `Image #1 · image/png` attaches (steps 1–8 pass), but the prompt fails with the provider error: the Oino Router account is out of OpenRouter credits (HTTP 402, `can only afford 25079` tokens) and the `ti` agent dir had no router model catalog (copied from this PC since). Not a GUI defect |
| `subagent-real`, `rewind-real` | real | — | **not run** — same provider block |

Screenshots: `docs/dev/v1/design/shots/desktop/<scenario>/*.png`.

Two harness limits found and handled:

1. A `FileDrop` event does not change GPUI's input modality, and hover (which `on_drop`
   listeners test) is suppressed while the last input was a keystroke — `drop` now sends a
   `MouseMove` first, like a real drag.
2. The transcript is virtualised: a user bubble scrolled above the viewport is not rendered,
   and the wheel injected through `dispatch_event` did not move the list on Xvfb, so the
   Rewind button of a long transcript is not clickable from a scenario (`rewind-real` uses a
   short real transcript instead).

## 8. Not covered

* **Mouse drag** (splitter resize, browser page interaction, drag-and-drop) — `click` presses
  and releases; a drag needs `MouseMove` with `pressed_button` between down and up.
* **Touch** (`TouchEvent`) — the driver speaks mouse + keyboard; the mobile shell's gestures are
  not injectable yet.
* **Multi-window**: the registry follows the main window only (the mascot window is ignored).
* **Embedded web content** (WebKitGTK) — the tree stops at the webview; page content is not
  readable or clickable through the RPC.
* **Real providers** — scenarios run on the `--demo` fixture or `--faux`; the driver itself is
  backend-agnostic, so a real session only needs `settings.json` defaults.
