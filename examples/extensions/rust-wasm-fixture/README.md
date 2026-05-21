# Oino Rust WASM extension fixture

This fixture is generated from `oino-extension-sdk::ExampleExtensionTemplate` and is kept as a human-readable authoring example.

It covers:

- model-visible tool contribution
- slash command contribution
- keymap contribution
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
