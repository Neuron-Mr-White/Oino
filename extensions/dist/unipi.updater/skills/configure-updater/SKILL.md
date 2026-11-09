---
name: configure-updater
description: Guide for using and configuring the Oino updater — release checks, changelog browser, and readme browser.
---

# Configure Updater

The `unipi.updater` extension checks GitHub for new Oino releases, shows what changed, and
browses the bundled READMEs and the changelog.

## Commands

### `/readme [package]`

Browse README.md files for Oino and its bundled/installed extensions.

- **No argument** — Opens a list of all packages with their versions. Select one to read its README.
- **With argument** (e.g., `/readme cron`) — Opens that package's README directly.
- The docs pseudo-package (`oino.docs`) carries the repository README.

### `/changelog`

Browse the changelog (bundled release notes, or fetched from GitHub releases) in a viewer.

- Shows a version list with status labels (✓ Current, ↑ New)
- Select a version to view its changelog details (Added, Fixed, Changed sections)
- Follows [Keep a Changelog](https://keepachangelog.com) format

### Update checks

The updater checks GitHub releases at session start and on a repeating timer, using the
configured check interval. When a new release is found it shows an update card / overlay;
you can skip that version — the skip is remembered and the prompt only re-appears for a
newer release. Installing the update is done through the app (update card / About page),
not by the extension.

## Settings

Configure in **Settings › Extensions › Updater**:

- **Check interval** (`checkIntervalMs`) — How often to check GitHub for a new Oino release
  (30 min / 1 hour / 6 hours / daily)
- **Auto update** (`autoUpdate`) — What happens when an update is found:
  - `disabled` — no update checks
  - `notify` — tell you; you install via the app (`auto` behaves the same as `notify`)

## Config File

Settings live in the `updater` namespace: `extensionSettings.updater` in
`~/.oino/settings.json`:

```json
{
  "extensionSettings": {
    "updater": {
      "checkIntervalMs": 3600000,
      "autoUpdate": "notify"
    }
  }
}
```

## Cache

A small last-check cache (`last-check.json` in the extension's cache dir):
- Tracks when the last check was performed
- Stores the latest version found
- Remembers skipped versions
- Re-prompts only when a newer version than the skipped one appears

## Navigation

The readme and changelog viewers are list + detail panels:
- ↑/↓ — Navigate the list
- `Enter` — Open a selection
- `Esc` — Back/close
