# ADR: Oino Extension Runtime ABI v1

Date: 2026-05-21
Status: Accepted

## Decision

Oino extension runtime v1 uses an Oino-owned `wasm-json-v1` ABI: lifecycle messages are JSON values carried over a host-owned runtime trait with explicit initialize, invoke, progress, cancel, shutdown, and structured-error operations.

The first implementation is `oino-extension-runtime` with a deterministic `JsonWasmRuntime` fixture/runtime boundary. It validates ABI name, runtime entry, object-shaped payloads, handler lookup, progress messages, cancellation, timeout, crash state, shutdown state, and denied host imports.

## Options Considered

1. **Simple Wasmtime JSON ABI**
   - Pros: small host-owned contract, easy to inspect, simple fixtures, quick path to untrusted WASM once Wasmtime is added behind the same trait.
   - Cons: less type-safe than component-model WIT and needs later performance tuning.

2. **WASI Preview 2 / Component Model**
   - Pros: strong interface definitions, portable capabilities, long-term ecosystem direction.
   - Cons: too much upfront complexity for Oino's first extension kernel; harder to dogfood quickly.

3. **Extism-style plugin runtime**
   - Pros: practical plugin lifecycle and SDK precedent.
   - Cons: additional runtime conventions and dependency choices before Oino's permission/capability broker is stable.

## Rationale

Oino needs a stable host contract before committing to a specific WASM engine integration. `wasm-json-v1` lets the extension manager, capability broker, tool bridge, hook runner, and tests agree on lifecycle semantics now, while keeping Wasmtime or another sandbox implementation behind a trait.

## Security Policy

Filesystem, shell/process, secrets, raw network, and unrestricted host imports are unavailable by default. Any privileged operation must flow through a named capability broker request. Unauthorized imports return structured errors and update runtime health instead of crashing Oino.

## Consequences

- The ABI is easy to test and dogfood.
- Future Wasmtime/component-model work can implement the same runtime trait or introduce a v2 ABI with migration.
- Extension authors initially target JSON schemas and named handlers rather than arbitrary host imports.
