# Oino Notify

`builtin:notify` enables host-owned ntfy notifications for selected Oino lifecycle events.

## Install

Open `/extensions`, choose **Install package**, and enter:

```text
builtin:notify
```

Then enable package `oino.notify` if it is not already enabled. When enabled, Oino shows `/settings notify` in command suggestions and the Notify page in the settings overlay.

## Configuration

Interactive configuration:

```text
/settings notify
```

Use `p` to edit project values, `g` to edit global values, `Enter` to toggle/edit the selected row, and `x` to clear a scoped value so project settings can inherit global defaults.

Notification settings also live in the normal Oino settings files:

- Global: `~/.oino/settings.json`
- Project: `<project>/.oino/settings.json`

Project settings override global settings field-by-field. Example:

```json
{
  "notify": {
    "enabled": true,
    "events": ["agent_end", "tool_error"],
    "ntfy": {
      "server": "https://ntfy.sh",
      "topic": "my-oino-topic",
      "token": "optional-access-token",
      "priority": "default",
      "tags": ["oino"]
    }
  }
}
```

`enabled` must be `true`, a `topic` is required, and the `oino.notify` package must be enabled. If `server` is omitted, Oino uses `https://ntfy.sh`.

Supported events:

- `agent_end` — send when an Oino run finishes.
- `tool_error` — send when a tool returns an error.

If `events` is omitted, both supported events are enabled.

## Safety

The package declares host `notify` capability and raw network access because the host posts to the configured ntfy server. It does not expose model-callable tools, shell commands, filesystem writes, provider mutation, or package-management permissions.
