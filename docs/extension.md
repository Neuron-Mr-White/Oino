# Extensions

Extensions add commands, tools, themes, prompts, skills, settings pages, and model providers to Oino.

## Open the extension manager

Inside Oino, run:

```text
/extensions
```

Use the manager to enable, disable, install, uninstall, and inspect extensions. If you install or update extensions outside the UI, run:

```text
/reload
```

## Install the built-in extension pack

The main installer enables built-ins automatically. From a source checkout you can refresh them manually:

```bash
bash scripts/install-all-builtins.sh      # macOS/Linux/Unix
.\scripts\install.ps1                    # Windows PowerShell, also builds/installs Oino
```

Then restart Oino or run `/reload`.

## Useful extension commands

```text
/extensions          open the extension manager
/extensions update   update installed extension packages from remembered sources
/reload              rescan extensions, prompts, skills, themes, and cached models
/auth                show extension auth/readiness status
/account             show provider/runtime status
```

## Safety model

Extensions are explicit. Oino loads from Oino-owned locations such as `~/.oino/extension-packages` and project `.oino/` paths. External agent folders are not loaded automatically.

When something looks wrong, open `/extensions` and check health, diagnostics, and whether the package is enabled globally or for the project.

## WASM extensions

Extensions with `runtime.kind: wasm` run as real Wasmtime components
(`oino:extension@0.1.0`). Their tools, commands, hooks, and providers work in
the TUI and one-shot `oino "prompt"` runs once the package is installed and
enabled.

Permissions are enforced from the manifest: filesystem preopens, network
hosts, declared secrets, allowlisted exec commands, and per-extension key/value
storage under `~/.oino/extension-data/<id>/`. Extensions that exceed limits or
trap repeatedly are marked unhealthy in `/extensions` instead of crashing
Oino. `host.confirm` is not wired yet — confirmation calls return declined
with a visible notice.
