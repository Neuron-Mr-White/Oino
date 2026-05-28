# Optional built-in extension packages

This directory contains Oino-owned extension packages that ship with the repository but are not enabled automatically. They are normal Oino packages: install them from `/extensions` with the `builtin:<alias-or-id>` source shorthand (or with this directory as a local source during development), then toggle their package, extension, or individual contributions at project/global scope.

These packages are intentionally kept as declarative Oino manifests and assets. Runtime behavior that needs privileged host integration remains in Oino core and is guarded by the same extension policy and permission surfaces as external packages.

Current packages:

| Package | Source path | Purpose |
|---|---|---|
| `oino.footer_status` (`builtin:footer-status`) | `extensions/built-in/footer-status` | Adds top and bottom composer-adjacent status surfaces for model, thinking, working directory, and context usage. |
| `oino.ralph_loop` (`builtin:ralph-loop`) | `extensions/built-in/ralph-loop` | Adds Oino-native Ralph loop auto-continuation, task/steering/history files, command metadata, promise tags, and operating skill assets. |
| `oino.mode_sandbox` (`builtin:mode-sandbox`) | `extensions/built-in/mode-sandbox` | Adds `/mode <profile>` sandbox switching with global defaults, project overrides, prompts, tool allow-lists, and a configuration skill. |
| `oino.notify` (`builtin:notify`) | `extensions/built-in/notify` | Adds ntfy notifications for selected Oino lifecycle events with `/settings notify` plus global/project config. |
| `oino.craft_skill` (`builtin:craft-skill`) | `extensions/built-in/craft-skill` | Adds an Oino-native skill for creating, improving, and validating reusable skills. |
| `oino.vcc` (`builtin:vcc`) | `extensions/built-in/vcc` | Adds deterministic `/compact`, `/recall`, and `vcc_recall` session-history surfaces. |
| `oino.ask_user` (`builtin:ask-user`) | `extensions/built-in/ask-user` | Adds the sequential `ask_user` tool backed by an Oino TUI questionnaire modal. |
