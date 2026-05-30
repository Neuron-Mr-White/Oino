# Built-in Extensions

Oino ships optional built-in extensions. The normal installer enables them automatically.

From a source checkout, install or refresh them manually with:

```bash
bash scripts/install-all-builtins.sh      # macOS/Linux/Unix
.\scripts\install.ps1                    # Windows PowerShell, also builds/installs Oino
```

Then run `/reload` or restart Oino.

## Included packages

| Package | What it adds |
| --- | --- |
| `oino.router` | `/router` setup, dashboard, status, model refresh, restart, and password reset commands. |
| `oino.footer_status` | Extra footer status lines for model, thinking, working directory, and context. |
| `oino.ralph_loop` | Iterative development loops controlled from inside Oino. |
| `oino.mode_sandbox` | `/mode <profile>` sandbox/profile controls. |
| `oino.notify` | Notification hooks for selected lifecycle events. |
| `oino.craft_skill` | Help creating Oino skills. |
| `oino.vcc` | Session compaction and recall. |
| `oino.ask_user` | Structured question tool with an Oino modal. |

## Managing them

Open:

```text
/extensions
```

Use the overlay to inspect health, toggle packages, and update installed packages.
