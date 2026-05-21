# Oino Extension SDK and Devkit

The first author-facing SDK is `crates/oino-extension-sdk`. It deliberately reuses Oino's core contracts instead of defining separate schemas.

## Rust WASM path

- Generate starter manifests with `oino-extension-devkit template-extension` and `template-package`.
- Validate author manifests/packages with `validate-extension` and `validate-package`.
- Use `WasmSdk` helpers for JSON-v1 tool/command outputs, host capability requests, UI state updates, and persistence capability payloads.
- Use `ExtensionTestHarness` for local tests covering runtime tool/command calls, host capability mocks, permission denials, UI state snapshots, persistence, and package validation.

Example fixture: `examples/extensions/rust-wasm-fixture`.

## Multi-language roadmap

- **TypeScript/JavaScript:** generate JSON schemas from the Rust contracts and provide a thin JSON-v1 helper package. No Pi API compatibility shim is planned.
- **Go:** provide typed structs for JSON-v1 request/response payloads and manifest generation after schema export stabilizes.
- **Python:** provide dataclass/Pydantic helpers for manifest validation and local fixture-runtime tests after schema export stabilizes.

All language bindings must pass the parity/contract tests in `oino-extension-sdk` before being documented as stable.
