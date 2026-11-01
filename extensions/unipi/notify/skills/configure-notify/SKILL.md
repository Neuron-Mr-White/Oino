---
name: configure-notify
description: >
  Help user configure Oino notification settings — platforms (native, Gotify, Telegram, ntfy),
  events, and per-event routing. Guide through setup or make changes directly.
---

# Configure Notify

Help users configure the `unipi.notify` notification system.

## When to use

- User asks to set up notifications
- User asks to enable/configure Gotify, Telegram, ntfy, or native notifications
- User wants to change which events trigger notifications
- User asks about notification settings

## Config locations

Settings live in the `notify` namespace of the Oino settings hub:

- **Global:** `extensionSettings.notify` in `~/.oino/settings.json`
- **Project:** `extensionSettings.notify` in `<project>/.oino/settings.json` (project wins)

Secret values (`gotify.appToken`, `telegram.botToken`, `telegram.chatId`, `ntfy.token`) are
stored in Oino's secret store, never in `settings.json` — set them through the Settings UI
or the setup wizards, not by editing the file.

The user-facing surface is **Settings › Extensions › Notify** (schema form) or `/settings`
in the terminal. It has pages for each platform, per-event switches, and General
(default platforms, native, recap) and Silence & renotify sections.

## Config structure

```json
{
  "defaultPlatforms": ["native"],
  "events": {
    "ralph_loop_end": { "enabled": true, "platforms": [] },
    "mcp_server_error": { "enabled": true, "platforms": [] },
    "agent_end": { "enabled": false, "platforms": [] },
    "agent_settled": { "enabled": true, "platforms": ["ntfy"] },
    "session_shutdown": { "enabled": false, "platforms": [] },
    "ask_user_prompt": { "enabled": false, "platforms": [] },
    "permission_request": { "enabled": false, "platforms": [] },
    "input_needed": { "enabled": true, "platforms": ["ntfy"] }
  },
  "native": {
    "enabled": true,
    "suppressWhenFocused": false
  },
  "recap": {
    "enabled": false,
    "model": null
  },
  "gotify": {
    "enabled": false,
    "serverUrl": null,
    "priority": 5
  },
  "telegram": {
    "enabled": false
  },
  "ntfy": {
    "enabled": false,
    "serverUrl": "https://ntfy.sh",
    "topic": null,
    "priority": 3
  },
  "silenceAfterInput": {
    "enabled": false,
    "windowMs": 10000,
    "platforms": ["native"]
  },
  "renotify": {
    "enabled": true,
    "intervalMs": 120000,
    "maxRepeats": 3
  }
}
```

## Platforms

### Native OS (default: enabled)

Desktop notifications through the desktop notification service. Works out of the box on
Windows, macOS, Linux. `native.suppressWhenFocused` holds native notifications while the
terminal has focus.

### Recap (default: disabled)

Periodic digests of what happened while the user was away, written by a model
(`recap.model`; empty = the session's model).

### Silence after input (default: off)

Quiet listed platforms for `windowMs` after any terminal keypress.

```json
"silenceAfterInput": {
  "enabled": true,
  "windowMs": 10000,
  "platforms": ["native"]
}
```

- `enabled` — master switch
- `windowMs` — quiet window in milliseconds (default: 10000)
- `platforms` — channels to silence (`native`, `gotify`, `telegram`, `ntfy`). Empty list silences all enabled platforms (same as `events.*.platforms`).

Configure in Settings › Extensions › Notify → Silence & renotify.

### Re-notify unanswered prompts (default: enabled)

When a blocking prompt (`ask_user_prompt`, `permission_request`, `input_needed`) is not
answered, notify re-sends the same notification every `intervalMs`, title suffixed
`(still waiting)`, priority `high`, up to `maxRepeats` times.

```json
"renotify": {
  "enabled": true,
  "intervalMs": 120000,
  "maxRepeats": 3
}
```

- `enabled` — master switch (default: true)
- `intervalMs` — delay between reminders in milliseconds, minimum 10000 (default: 120000 = 2 min)
- `maxRepeats` — reminders after the first notification, 0 sends none (default: 3)

Reminders stop as soon as the user presses a key, the agent starts a new turn, or the
session ends. Arming a new prompt replaces any existing reminder (only one can be
outstanding). Reminders bypass `silenceAfterInput` since blocking events are exempt.

Configure in Settings › Extensions › Notify → Silence & renotify.

### Gotify (default: disabled)

Self-hosted push notification server. Requires:
- `serverUrl` — URL of your Gotify server (e.g. `https://gotify.example.com`)
- `appToken` — Application token from Gotify web UI (Apps → Create Application; stored as a secret)
- `priority` — 1-10 (default: 5)

**Setup options:**
1. **Setup wizard:** Settings › Extensions › Notify → Platforms → Gotify → "Setup wizard…" for guided setup with a connection test
2. **Settings form:** fill the fields in Settings › Extensions › Notify
3. **Agent can write config:** merge non-secret changes into `extensionSettings.notify`; the token goes through the wizard or the masked Settings field

### Telegram (default: disabled)

Bot API notifications. Requires:
- `botToken` — From @BotFather (stored as a secret)
- `chatId` — Auto-detected by the Telegram setup wizard

**Setup options:**
1. **Setup wizard:** Settings › Extensions › Notify → Platforms → Telegram → "Setup wizard…"
2. **Settings form** or agent merge, as with Gotify

### ntfy (default: disabled)

Simple HTTP-based pub-sub notification service. Supports public [ntfy.sh](https://ntfy.sh) and self-hosted instances.
Requires:
- `serverUrl` — ntfy server URL (default: `https://ntfy.sh`)
- `topic` — Topic name to publish to (acts as the channel)
- `token` — Optional access token for authenticated servers (stored as a secret)
- `priority` — 1-5 (default: 3)

**Setup options:**
1. **Setup wizard:** Settings › Extensions › Notify → Platforms → ntfy → "Setup wizard…" for guided setup with a connection test
2. **Settings form:** fill `ntfy.*` in Settings › Extensions › Notify
3. **Agent can write config:** merge non-secret changes into `extensionSettings.notify`; the token goes through the wizard or the masked Settings field

**Scope:** put `ntfy.*` in the project's `.oino/settings.json` for a per-project topic;
the global `extensionSettings.notify` applies everywhere else (project wins).

## Commands

| Command | Description |
|---------|-------------|
| Settings › Extensions › Notify (`/settings`) | Toggle platforms and events, run the wizards |
| Notify → Platforms → Gotify → "Setup wizard…" | Interactive Gotify setup with connection test |
| Notify → Platforms → Telegram → "Setup wizard…" | Interactive Telegram setup |
| Notify → Platforms → ntfy → "Setup wizard…" | Interactive ntfy setup |
| Notify → Events → "Send test notification" | Send test notification to all enabled platforms (`/notify-test`) |

`/notify-event` lists and describes the notification events.

## Events

| Event | Default | Description |
|-------|---------|-------------|
| `ralph_loop_end` | On | A ralph loop iteration ends |
| `mcp_server_error` | On | An MCP server errors |
| `agent_end` | Off | A run finishes (once per run, after every follow-up and continuation, not per message) |
| `agent_settled` | On (ntfy) | A run finishes (same single notification as `agent_end`; either switch turns it on) |
| `session_shutdown` | Off | The session shuts down |
| `ask_user_prompt` | Off | The agent asks you a question and is waiting for an answer |
| `permission_request` | Off | A permission prompt is about to be shown |
| `input_needed` | On (ntfy) | The agent waits on any prompt while it runs (covers tools that send no event of their own) |

Each event can override `platforms` — empty array means use `defaultPlatforms`.
Old `workflow_end` and `memory_consolidated` events no longer exist.

`ask_user_prompt`, `permission_request` and `input_needed` are **blocking** events: while
one is unanswered the agent is parked, so notify re-sends it periodically (see the
Re-notify unanswered prompts section under Platforms).

### `permission_request`

Fires immediately before the user-facing permission dialog is invoked. Policy auto-allow,
policy deny, and session-approved rules do **not** emit it, so this event does not produce
notification spam. Forwarded subagent prompts are handled too — the notification is
suffixed with `(forwarded)`.

The notification is formatted defensively:

```text
Oino — Permission Request
Current agent requested bash 'npm test'. Allow this command?
```

Enable it when you run Oino in a background pane and don't want to miss a permission prompt.

## Agent workflow

### Reading current config

```bash
python3 -c 'import json,sys;print(json.dumps(json.load(open(sys.argv[1])).get("extensionSettings",{}).get("notify",{}),indent=2))' ~/.oino/settings.json
```

### Updating config programmatically

Read `extensionSettings.notify` from `~/.oino/settings.json`, merge the changes, write it
back (Oino re-reads settings on start). Example — enable Gotify:

```json
{
  "gotify": {
    "enabled": true,
    "serverUrl": "https://gotify.example.com",
    "priority": 7
  }
}
```

Then have the user enter `gotify.appToken` in Settings › Extensions › Notify (masked
field) or via the setup wizard — never put secret values into `settings.json`.

### Guiding user to interactive setup

For Gotify: Settings › Extensions › Notify → Platforms → Gotify → "Setup wizard…"
For Telegram: Settings › Extensions › Notify → Platforms → Telegram → "Setup wizard…"
For ntfy: Settings › Extensions › Notify → Platforms → ntfy → "Setup wizard…"
For general settings: Settings › Extensions › Notify

## Validation rules

- Gotify: `serverUrl` and `appToken` required when enabled
- Gotify: `priority` must be 1-10
- Telegram: `botToken` and `chatId` required when enabled
- ntfy: `serverUrl` and `topic` required when enabled
- ntfy: `priority` must be 1-5
- renotify: `intervalMs` must be a finite number >= 10000 (else the default 120000 is used)
- renotify: `maxRepeats` must be an integer >= 0 (else the default 3 is used)
