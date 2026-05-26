# Extension Development

Extensions are packages with one `oino.package.json` and one or more `oino.extension.json` manifests.

## Validate an extension package

```bash
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- validate-package examples/extensions/rust-wasm-fixture
```

## Package shape

```text
my-package/
  oino.package.json
  extensions/
    my-extension/
      oino.extension.json
      ...assets or wasm files...
```

## What extensions can contribute

- Slash commands
- Tools
- Settings pages
- Themes
- Keymap entries
- Prompts and skills
- Provider/runtime metadata
- Hooks and diagnostics

## Local testing loop

1. Edit the package.
2. Run the devkit validator.
3. Install or copy it into `~/.oino/extension-packages/`.
4. Run `/reload`.
5. Open `/extensions` and check diagnostics.

Keep extension IDs stable and prefer explicit permissions. Oino does not implicitly load unrelated agent extension folders.
