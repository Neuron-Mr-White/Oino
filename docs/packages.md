# Packages

An extension package is a directory:

```text
my-ext/
  extension.json      # manifest
  component.wasm      # the component
  skills/ prompts/ themes/ docs/   # optional resources declared in the manifest
```

Minimal manifest:

```json
{
  "apiVersion": "oino.extension/1",
  "id": "acme.hello",
  "name": "Hello",
  "version": "0.1.0",
  "kind": "extension",
  "description": "Says hello.",
  "permissions": { "network": { "hosts": ["api.example.com"] } },
  "resources": { "skills": ["skills"] }
}
```

`kind: "provider"` packages must list `provides.apis` and export the `provider` interface.
Fields, permissions and validation rules: [dev/v1/extension-api.md](dev/v1/extension-api.md) §7.

## Installing

Copy (or symlink) the package directory into `~/.oino/extensions/` (global) or
`<project>/.oino/extensions/` (project; needs trust), or pass `-e <dir>` for one run.
`cargo xtask wasm` produces ready-made packages under `extensions/dist/`.

The pi-style package manager installs from a directory, an archive, an `https` archive or a
`git+https` URL: `oino install <source> [--project] [--yes]`, `oino remove <id>`,
`oino update <id>…` / `oino update --extensions`, `oino list`, `oino config`. `oino ext
validate` is not available yet.
