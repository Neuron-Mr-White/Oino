# Oino Extension User Guide

This guide explains how to find, install, enable, update, and remove Oino extensions from the user-facing `/extensions` panel.

Oino extensions are **Oino-native packages**. They are not arbitrary npm packages and they do not use unrelated agent extension APIs. A package must contain an `oino.package.json` manifest at the install root.

## Quick start

Run Oino and open the extension manager:

```text
/extensions
```

Install the bundled example into the current project:

```text
i
examples/extensions/rust-wasm-fixture
Enter
```

Install from a GitHub repository instead of a local path:

```text
i
owner/repo
Enter
```

or:

```text
i
https://github.com/owner/repo.git
Enter
```

After install, the package is enabled in the selected scope and Oino reloads extensions automatically. Stay in `/extensions` to inspect the package, extension, and contribution rows.

## Install sources

The `/extensions` panel accepts these package source formats:

| Source | Example | Notes |
|---|---|---|
| Local relative path | `examples/extensions/rust-wasm-fixture` | Resolved relative to the current project/workdir. |
| Local absolute path | `/home/me/dev/my-extension` | Must contain `oino.package.json`. |
| Home path | `~/oino-extensions/my-extension` | `~` expands to the Oino user's home directory. |
| GitHub shorthand | `owner/repo` | Clones `https://github.com/owner/repo.git`. If a matching local path exists, Oino treats it as local. |
| Explicit GitHub shorthand | `github:owner/repo` or `gh:owner/repo` | Recommended when you want to be unambiguous. |
| GitHub URL | `https://github.com/owner/repo.git` | `.git` is optional for GitHub HTTPS URLs. |
| Generic Git URL | `git+https://host/owner/repo.git`, `ssh://...`, `git@github.com:owner/repo.git` | Requires `git` on `PATH`. |
| Branch/tag | `github:owner/repo#v1.2.3` | Uses `git clone --branch`; intended for branch or tag names. |

Git installs clone the repository into a temporary checkout, validate/install the package, then delete the checkout. The repository root must be an Oino package root with `oino.package.json`.

## Project vs global install

In `/extensions`:

- `i` installs into the **project** scope.
- `I` installs into the **global** scope.

Project installs are copied under:

```text
<project>/.oino/extension-packages/<package-id>/
```

Global installs are copied under:

```text
~/.oino/extension-packages/<package-id>/
```

Use project installs when the extension is specific to one repository. Use global installs when you want the extension available across Oino projects.

## Panel controls

Open:

```text
/extensions
```

The panel has two tabs:

- **Manage** — one row per installed package for install, update-by-reinstall, uninstall, and coarse enablement.
- **Registered** — extension runtime rows plus individual registered contributions such as tools, commands, UI surfaces, themes, resources, hooks, providers, autosuggest, renderers, diagnostics, health, and persistence.

Controls:

| Key | Action |
|---|---|
| `Tab` / `Shift-Tab` | Switch between **Manage** and **Registered** tabs. |
| `1` | Open **Manage**. |
| `2` | Open **Registered**. |
| `/` | Search within the current tab. |
| `↑` / `↓` | Move selection. |
| `i` | Install a package into the current project. |
| `I` | Install a package globally. |
| `Enter` while install input is active | Install the typed source. |
| `Esc` while install input is active | Cancel install input. |
| `p` or `Enter` on a row | Toggle project enablement for the selected package/extension/contribution. |
| `g` on a row | Toggle global enablement for the selected package/extension/contribution. |
| `o` on a contribution row | Prefer this contribution as the project conflict winner. |
| `O` on a contribution row | Prefer this contribution as the global conflict winner. |
| `c` on a contribution row | Clear the project conflict override for this contribution id. |
| `C` on a contribution row | Clear the global conflict override for this contribution id. |
| `u` or `x` on a package row | Start uninstall confirmation. |
| `Enter` or `y` during uninstall confirmation | Remove the selected installed package. |
| `n` or `Esc` during uninstall confirmation | Cancel uninstall. |
| `Esc` | Close the panel when not in install/confirm mode. |

## Extension surface controls

Oino owns extension surface navigation. Extensions can register sidebar, main panel, footer/status, header, composer widget, floating, editor-metadata, and working-indicator surfaces, but the host controls focus, tab switching, hide/show, and close behavior.

Global shortcuts:

| Key | Action |
|---|---|
| `Ctrl-O Tab` | Focus the next visible extension surface slot. |
| `Ctrl-O Shift-Tab` | Focus the previous visible extension surface slot. |
| `Ctrl-O ]` | Switch to the next tab in the focused extension slot. |
| `Ctrl-O [` | Switch to the previous tab in the focused extension slot. |
| `Ctrl-O w` | Close/hide the focused extension surface slot. |
| `Esc` | Also closes the focused extension surface slot before falling through to normal app behavior. |
| `Ctrl-O b` | Toggle all extension sidebar slots. |
| `Ctrl-O m` | Toggle all extension main-panel slots. |

If several extensions register the same slot, Oino shows a tab row and lets you switch the active one instead of silently letting extensions override each other.

## What install does

When you install a package, Oino:

1. Resolves the source as a local path or Git/GitHub repository.
2. Clones Git sources into a temporary directory if needed.
3. Validates `oino.package.json` and referenced extension manifests.
4. Checks Oino compatibility, package dependencies, scope, permissions, and trust metadata.
5. Copies the package into the selected project/global package directory.
6. If the package id is already installed in that scope, treats install as an update attempt.
7. Enables the package in the selected scope's settings.
8. Reloads the Extension Manager snapshot.
9. Refreshes model-visible tools, slash commands, prompt/skill resources, UI surfaces, keymaps, themes, providers, autosuggest entries, and management rows.

## Updating extensions

For now, update by installing the same package id again from the newer source:

```text
/extensions
i
github:owner/repo#v1.2.4
Enter
```

If the package id already exists in the target scope, Oino invokes the update path. The package manifest version should be newer or otherwise compatible with the lifecycle service's update checks.

## Removing extensions

1. Open `/extensions`.
2. Select the package row, not a contribution row.
3. Press `u` or `x`.
4. Press `Enter`/`y` to confirm.

Oino removes the installed package from the selected package scope and disables that package in settings. Contributions disappear after the extension snapshot reloads.

## Enablement levels

Oino displays three broad row types:

- **Package** — install unit copied under `.oino/extension-packages`.
- **Extension** — runtime/manifest unit inside a package.
- **Contribution** — specific tool, command, UI surface, keymap, theme, provider metadata, etc.

Each row shows `G:ON/OFF` and `P:ON/OFF`:

- `G` is the global setting.
- `P` is the project setting.

Project settings override global defaults for the current project. If a globally installed package should be unavailable in one project, select its package/extension/contribution row and toggle project off with `p`.

Contribution rows may also show override badges:

- `OVR:G` — this candidate is the global conflict winner.
- `OVR:P` — this candidate is the project conflict winner.
- `OVR:G/P` — both scopes prefer this candidate.

Use `o`/`O` to choose a conflict winner and `c`/`C` to clear the override. This works across contribution families such as commands, tools, keymaps, UI surfaces, resources/skills, providers/models, themes, hooks, autosuggest, renderers, diagnostics, health, and persistence.

## Trust and permissions

Before installing community packages, inspect the repository and manifest:

- `oino.package.json` package id, publisher, version, docs, dependencies, trust metadata.
- `oino.extension.json` permissions and contributions.
- Requested host capabilities, persistence scopes, network/process/secrets permissions.

Oino validates and brokers privileged runtime capabilities, but users should still treat community packages as code. Prefer reviewed packages and pinned tags:

```text
github:owner/repo#v1.2.3
```

## Troubleshooting

### `git clone failed`

- Ensure `git` is installed and on `PATH`.
- Check network access.
- For private repos, ensure your SSH key or credential helper works outside Oino.
- Prefer explicit SSH URLs for private GitHub repos: `git@github.com:owner/repo.git`.

### `Extension install failed: ... missing manifest`

The install root must contain `oino.package.json`. If the repo stores the package in a subdirectory, clone it manually and install the subdirectory as a local path until subdirectory Git install support lands.

### Installed package does not appear

- Reopen `/extensions` or use `/reload` if needed.
- Confirm you installed into the scope you are viewing.
- Confirm package id is not disabled at project scope.
- Look for diagnostics rows/messages in `/extensions`.

### Tool is visible but execution is a fixture response

The current extension runtime path includes deterministic fixture/runtime support. Production untrusted WASM execution remains a hardening follow-up; extension contributions can be visible before real WASM execution is fully enabled.
