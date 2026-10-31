# oino-resources

Resource loading for Oino: context files (`AGENTS.md` & co.), SYSTEM/APPEND prompt sources,
skills, prompt templates, theme files, the structured system prompt, project trust and the
keybindings file.

Synchronous std I/O, no async runtime; `oino-core` calls it on blocking threads. Packages and
extensions are resolved by `oino-core` and contribute paths through `ResourceLoader::extend`.

* `loader` — `ResourceLoader` with reload and extension-contributed paths
* `discovery` — settings arrays plus auto-discovery with `!glob`/`+path`/`-path`
* `system_prompt` — structured, section-wrapped system prompt build and diffing
* `skills`, `templates`, `context_files` — skill expansion, `/prompt` templates, project context
* `trust` — project trust decision and `trust.json` storage
