# Oino Rust WASM extension fixture

This fixture is generated from `oino-extension-sdk::ExampleExtensionTemplate` and is kept as a human-readable authoring example.

It covers:

- model-visible tool contribution
- slash command contribution
- keymap contribution (`Ctrl-O x` in the default keymap)
- sidebar, floating panel, and footer UI surfaces
- theme tokens
- autosuggest provider
- provider/model metadata
- mutable tool-call hook
- project-scoped persistence

Validate it with:

```bash
cargo run -p oino-extension-sdk --bin oino-extension-devkit -- validate-package examples/extensions/rust-wasm-fixture
```

To try the keymap contribution, install the fixture, close `/extensions`, and press `Ctrl-O x` from the main chat view. If you changed keymaps, check `/help` or `/settings keymaps` for the current binding.

Learn more:

- [Install and inspect this fixture](../../../docs/extension-kernel/user-guide.md#quick-start)
- [Build an extension package](../../../docs/extension-kernel/developer-guide.md)
- [SDK and devkit reference](../../../docs/extension-sdk/README.md)
