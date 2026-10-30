# Extensions

Oino extensions are **WebAssembly components** implementing the WIT package
`oino:extension@1.0.0` ([extension-api.md](extension-api.md)). They play the role of pi's
TypeScript extensions: they subscribe to session events, register tools, slash commands,
shortcuts, flags, renderers, settings namespaces, providers and virtual models, and talk to
the user through frontend-agnostic UI requests. pi npm packages (TypeScript) cannot be loaded;
their Markdown resources (skills, prompts) can.

## Where extensions come from

Load order (later packages with the same id replace earlier ones in place):

1. **Distribution** — the share dir the installer owns (`/usr/share/unipi-oino/extensions`,
   `UniPi.app/Contents/Resources/extensions`, `%LOCALAPPDATA%\UniPi\extensions`; resolved
   relative to the binary, overridable with `OINO_DIST_EXTENSIONS`). Nothing is compiled
   into the binary.
2. **Global** — `~/.oino/extensions/<package>/extension.json` (a user copy shadows the
   distribution one).
3. **Project** — `<project>/.oino/extensions/<package>/` (trusted projects only).
4. **Command line** — `-e <package dir>` (repeatable, wins over everything).

`-ne/--no-extensions` skips every `kind: "extension"` package (providers stay loaded, explicit
`-e` packages still load). `--exclude-extension <id>` drops composed packages by id —
repeatable, comma-separated, applied after load-order replacement and before instantiation —
including explicit `-e` paths and providers (`builtin:` test components are not subject). An
id that matches nothing warns and is ignored; exclusion never writes settings.
`disabledExtensions` in settings disables ids persistently; the CLI
flag is an ephemeral union with it for one run (excluded either way: not loaded).

## Safe mode

After two consecutive failed starts (services or session creation errors, fatal startup
diagnostics, or a process that died mid-start), the next start runs with extensions off and
providers on — the agent stays usable — and a startup warning banner names the recorded
suspects: `Safe mode: extensions are off after 2 failed starts; suspects: unipi.notify (…)`.
A successful start resets the counter. The state lives in
`~/.oino/state/oino.safe-mode.json`; `_oino/hello` reports it as `safeMode`. Manual levers
are unaffected: `-ne` and `--exclude-extension` work as above in and out of safe mode.

## Sandbox

Each component runs in wasmtime with only the capabilities its manifest asks for
(`permissions`: network hosts, process spawning, file system preopens, secrets, nested agents,
…). Bundled components are trusted with the grants they declare; installed packages get what
their manifest lists, nothing more. Long tools (`ask_user`, subagents) run as async guest
tasks, so event handlers of the same extension keep working meanwhile.

Extension state lives in `~/.oino/state/<extension-id>/{global,workspace/<wsid>,session/<sid>}`
(`wsid` = first 16 hex chars of the SHA-256 of the project root). Nothing is written into your
project unless a tool does it.

## Writing an extension

Use the Rust guest SDK (`extensions/sdk/oino-extension`, `#[derive]`-style macros in
`oino-extension-macros`; providers use `oino-provider-kit`):

```bash
cargo new --lib my-ext   # inside extensions/ (members: sdk/*, providers/*, unipi/*, fixtures/*)
cargo xtask wasm -p my-ext
oino -e extensions/dist/my.ext
```

`extensions/fixtures/` holds small complete examples (`echo`, `ui-probe`, `async-probe`,
`provider-faux`); `extensions/unipi/*` are full-size ones. Packaging and manifests:
[packages.md](packages.md).
