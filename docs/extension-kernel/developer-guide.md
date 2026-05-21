# Oino Extension Developer Guide

This guide is for extension authors and Oino core developers building Oino-native extensions. It covers package layout, manifests, contributions, permissions, local testing, and installation from local paths or GitHub repositories.

Oino extension APIs are **Oino-native**. They intentionally do not implement Pi's TypeScript extension API. The stable contract is the manifest + registry + `wasm-json-v1` runtime boundary owned by Oino.

## Development prerequisites

From the Oino repository/worktree:

```bash
cargo --version
git --version
mise --version   # optional but recommended for project tasks
```

Useful validation commands:

```bash
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- validate-package examples/extensions/rust-wasm-fixture
cargo test -p oino-extension-sdk --no-fail-fast
mise run quality
```

## Mental model

An Oino extension package has three layers:

1. **Package** — installable unit with `oino.package.json`.
2. **Extension** — runtime unit with `oino.extension.json`.
3. **Contributions** — declarative tools, commands, UI surfaces, keymaps, themes, provider metadata, autosuggest entries, hooks, resources, diagnostics, health, and persistence declarations.

The host owns rendering, policy, settings, discovery, and capability enforcement. Extension code declares contributions and sends validated JSON-v1 runtime messages; it does not draw directly into Ratatui.

## Recommended package layout

A minimal package should look like this:

```text
my-oino-extension/
├── oino.package.json
├── extensions/
│   └── my-extension/
│       └── oino.extension.json
├── plugin.wasm              # current fixture/runtime entry path
├── docs/
│   └── README.md
├── examples/
│   └── basic.md
└── themes/                  # optional
    └── my-theme.json
```

For Git/GitHub installs, keep `oino.package.json` at the repository root:

```text
https://github.com/owner/my-oino-extension
└── oino.package.json
```

If your source repo is a monorepo, install from a local checkout subdirectory for now:

```bash
git clone https://github.com/owner/monorepo.git
# In Oino: /extensions → i → ../monorepo/path/to/oino-package
```

## Create a starter package

Generate templates:

```bash
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- template-package > oino.package.json
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- template-extension > extensions/my-extension/oino.extension.json
```

The checked-in example is a better full reference:

```text
examples/extensions/rust-wasm-fixture
```

Validate it:

```bash
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- validate-package examples/extensions/rust-wasm-fixture
```

## Package manifest: `oino.package.json`

The package manifest describes the install unit. Important fields:

```json
{
  "id": "acme.example_extension",
  "display_name": "Example Extension",
  "version": "1.0.0",
  "oino": "^0.1",
  "publisher": "acme",
  "description": "Adds an example tool and UI surface",
  "source": "https://github.com/acme/example-extension",
  "extensions": [
    {
      "manifest": "extensions/example/oino.extension.json",
      "enabled_by_default": true
    }
  ],
  "dependencies": [],
  "permissions": {
    "tools": ["example_tool"],
    "commands": ["example_command"],
    "host_capabilities": ["host.test.echo"],
    "ui": ["sidebar"],
    "filesystem": [],
    "shell_process": { "allowed": false, "commands": [] },
    "network": { "raw_network": false, "hosts": [] },
    "secrets": [],
    "session_persistence": [],
    "provider_mutation": [],
    "package_management": []
  },
  "trust": {
    "reviewed": false,
    "publisher": "acme",
    "advisories": []
  }
}
```

Guidelines:

- Use reverse-DNS or publisher-prefixed ids: `acme.example_extension`.
- Keep package ids stable forever; update `version` for releases.
- Keep package-level permissions at least as broad as included extension permissions.
- Include docs and examples paths where possible.
- Set `source` to the canonical repository URL for GitHub installs.
- Pin dependencies and document compatibility if your package requires another extension package.

## Extension manifest: `oino.extension.json`

The extension manifest describes one runtime unit and its contributions:

```json
{
  "id": "acme.example_extension",
  "package_id": "acme.example_extension",
  "display_name": "Example Extension",
  "version": "1.0.0",
  "oino": "^0.1",
  "protocol": 1,
  "runtime": {
    "kind": "wasm",
    "entry": "plugin.wasm",
    "abi": "wasm-json-v1"
  },
  "permissions": {
    "tools": ["example_tool"],
    "commands": ["example_command"],
    "host_capabilities": ["host.test.echo"],
    "ui": ["sidebar"],
    "filesystem": [],
    "shell_process": { "allowed": false, "commands": [] },
    "network": { "raw_network": false, "hosts": [] },
    "secrets": [],
    "session_persistence": [],
    "provider_mutation": [],
    "package_management": []
  },
  "contributes": {
    "tools": [
      {
        "id": "example_tool",
        "description": "Echo a message",
        "input_schema": {
          "type": "object",
          "properties": { "message": { "type": "string" } },
          "required": ["message"]
        },
        "execution_mode": "parallel",
        "handler": "handle_tool"
      }
    ],
    "commands": [
      {
        "id": "example_command",
        "description": "Run the example command",
        "handler": "handle_command"
      }
    ],
    "ui_surfaces": [],
    "keymaps": [],
    "hooks": [],
    "themes": [],
    "providers": [],
    "resources": [],
    "persistence": []
  }
}
```

Guidelines:

- Every contribution id must be valid and should be namespaced enough to avoid collisions.
- Every tool/command contribution must be covered by permissions.
- Prefer `execution_mode: "parallel"` only for side-effect-safe tools.
- UI contributions must be declarative; Oino validates state shape and renders host-side.
- Add conflict policy when a contribution may overlap with others.

## Contribution families

Supported contribution families include:

- **Tools** — model-visible callable tools with JSON input schemas.
- **Commands** — slash commands bridged into Oino command handling.
- **Hooks** — host lifecycle or tool-call hooks with priority and mutable/read-only mode.
- **UI surfaces** — sidebar, floating panel, footer/status, overlays, settings pages, health summaries.
- **Keymaps** — default bindings for extension actions.
- **Themes** — theme token overrides.
- **Providers/models** — provider/model metadata, privacy and capability descriptors.
- **Autosuggest** — composer suggestion providers.
- **Renderers** — transcript/message/tool renderer descriptors.
- **Resources** — prompt, skill, or other resource descriptors.
- **Persistence** — typed extension-owned persistence records.
- **Diagnostics/health** — health metadata and actionable diagnostics.

## Runtime and capability rules

The current ABI is `wasm-json-v1`:

- initialize
- invoke
- progress
- cancel
- shutdown
- health

Host capabilities are brokered. Do not assume raw filesystem, process, network, secrets, provider mutation, or persistence access. Declare permissions, then request capabilities through the runtime boundary. The broker audits, times out, size-limits, and denies calls that exceed policy.

Current built-in capability fixtures include:

- `host.test.echo`
- `host.web.search` fixture
- `host.persistence.read`
- `host.persistence.write`
- `host.persistence.delete`

## Local validation loop

Validate the package after every manifest change:

```bash
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- validate-package path/to/package
```

Validate one extension manifest:

```bash
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- validate-extension path/to/oino.extension.json
```

Run SDK and harness tests:

```bash
cargo test -p oino-extension-sdk --no-fail-fast
```

Run full Oino quality before proposing a package/kernel change:

```bash
mise run quality
```

## Test harness strategy

Use `oino-extension-sdk::ExtensionTestHarness` for deterministic tests. Cover:

- package/manifest validation
- tool invocation payloads and outputs
- command invocation payloads and outputs
- capability allow/deny behavior
- UI state snapshots and validation failures
- persistence read/write/delete payloads
- safe-mode behavior
- conflict/diagnostic output

Recommended test naming:

```text
extension_<feature>_validates_manifest
extension_<feature>_tool_payload_round_trips
extension_<feature>_denies_undeclared_capability
extension_<feature>_surface_state_matches_schema
```

## Install while developing

Install a local checkout into the current project:

```text
/extensions
i
../my-oino-extension
Enter
```

Install globally:

```text
/extensions
I
../my-oino-extension
Enter
```

After install, inspect rows in `/extensions`:

- package row health and diagnostics
- extension row lifecycle and permission summary
- contribution rows and conflicts
- `G:ON/OFF` and `P:ON/OFF` enablement

Use `p`/Enter to toggle project enablement and `g` to toggle global enablement.

## Publish through GitHub

For community/core developer sharing today, publish an Oino package repository:

1. Keep `oino.package.json` at repo root.
2. Include referenced extension manifests and assets.
3. Include `docs/README.md` explaining behavior and permissions.
4. Validate the package from a fresh clone.
5. Tag releases, e.g. `v1.0.0`.
6. Share a pinned install source:

```text
github:owner/repo#v1.0.0
```

Users can install with:

```text
/extensions
i
github:owner/repo#v1.0.0
Enter
```

For private repos, users can install via SSH if their local git credentials work:

```text
git@github.com:owner/private-extension.git#v1.0.0
```

## Update releases

To release an update:

1. Update code/manifests.
2. Bump package and extension versions.
3. Re-run validation and tests.
4. Commit and tag the repo.
5. Ask users to install the newer tag from `/extensions`.

Oino treats installing an already-installed package id in the same scope as an update path.

## Uninstall and cleanup

In `/extensions`, select the package row and press `u` or `x`. Confirm with `Enter`/`y`.

Uninstall removes the installed package directory and disables the package in settings. Extension-owned persistence cleanup is governed by each persistence contribution's cleanup policy.

## Security checklist before sharing

- [ ] Manifest ids are stable and namespaced.
- [ ] Requested permissions are minimal.
- [ ] No raw process/network/secrets permissions unless absolutely necessary.
- [ ] Docs explain all model-visible tools and side effects.
- [ ] Package source points to the public repo.
- [ ] Release tags are immutable in practice.
- [ ] Advisories/deprecation metadata are filled when relevant.
- [ ] Validation passes from a clean checkout.

## Common issues

### Package validates locally but not from GitHub

Check case-sensitive paths in `oino.package.json`. GitHub installs clone on the user's platform and then validate relative paths from the repository root.

### Contribution is pending review

External contributions default safe. Enable the package/extension/contribution from `/extensions` with project/global toggles.

### Tool appears but runtime behavior is fixture-like

The current branch includes deterministic fixture runtime support and kernel APIs. Production untrusted WASM runtime hardening is still a follow-up, so UI/tool visibility can precede final WASM execution behavior.

### Repo has the package in a subdirectory

For now, clone manually and install the subdirectory as a local path. Keep standalone extension package repos simple with `oino.package.json` at root for the best user experience.
