# wp-zvec round 3: release size decision

## Decision: keep zvec OFF in every release

The supervisor cutoff is 40 MB of incremental installed size. The measured GNU/Linux host plus stripped required native libraries adds **85,055,680 bytes (85.056 MB / 81.115 MiB)**. Including the Linux-installed CLI adds **110,728,728 bytes (110.729 MB / 105.599 MiB)**. Both exceed 40 MB, even before attribution files. Model weights and project indices are excluded: weights remain downloaded only after consent.

No release script was changed to enable zvec. `scripts/unipi-build.sh`, `apps/unipi/src-tauri/unipi-host-sidecar.sh`, and `.github/workflows/release.yml` retain feature-OFF Linux/macOS/Windows builds. No deb or other installer was built by this package.

## Measurements

Linux x86_64, Rust 1.99.0, Cargo release profile (`opt-level=3`, no custom LTO), same profile used by `scripts/unipi-build.sh`. Copies were stripped with GNU strip: `--strip-all` for executables, `--strip-unneeded` for native shared libraries. Exact bytes and SHA256s are in `evidence/zvec-size/sizes.json`; build/test logs are adjacent. Large binary copies remain under ignored `scratch/wp-zvec-size/`.

| Artifact | Feature OFF, bytes | Feature ON, bytes | Added bytes |
|---|---:|---:|---:|
| GNU host | 86,912,640 | 109,947,128 | 23,034,488 |
| GNU CLI | 62,054,024 | 87,727,072 | 25,673,048 |
| musl sidecar | 82,702,328 | build fails (upstream unsupported) | unavailable |
| libzvec_c_api.so | 0 | 41,462,640 | 41,462,640 |
| libonnxruntime.so.1 | 0 | 20,558,552 | 20,558,552 |

Runtime libraries counted once, not once per executable. ONNX Runtime is loaded dynamically; libzvec is an ELF NEEDED dependency (`host-needed.txt`). The host runs `--version` when the measured library directory is supplied via LD_LIBRARY_PATH. No CUDA/Vulkan Cargo features were enabled. The CPU llama SDK links statically; its contribution is already in executable bytes. Existing system C/C++ libraries are excluded from the incremental installed footprint.

Commands (from repo root, all CARGO_BUILD_JOBS=8):

```sh
cargo build --manifest-path apps/unipi/Cargo.toml --release -p unipi-host
cargo build --manifest-path apps/unipi/Cargo.toml --release -p unipi-host --features zvec
cargo build --release -p oino-cli --bin oino
cargo build --release -p oino-cli --bin oino --features oino-core/zvec
cargo zigbuild --manifest-path apps/unipi/Cargo.toml --release -p unipi-host --target x86_64-unknown-linux-musl
cargo zigbuild --manifest-path apps/unipi/Cargo.toml --release -p unipi-host --target x86_64-unknown-linux-musl --features zvec
```

The exact release sidecar target was attempted both ways. Feature ON fails in upstream zg-engine's build script: `No official ORT cpu distribution for x86_64-unknown-linux-musl`. Therefore there is no honest feature-enabled musl size comparison. The successful GNU release comparison is reported explicitly, not substituted silently for musl. Required stripped runtimes alone total 62,021,192 bytes, so the OFF decision does not depend on this limitation. Supporting feature-enabled musl would require a separate native-runtime port, outside this size decision.

## Capability-based Settings

The running host now answers host-only `search_capabilities` with `{ "semanticSearch": true|false }`, derived from its compiled Cargo feature, not the saved toggle. Settings › Search queries the selected host and **does not render** the semantic toggle, consent checkbox, or Build index when unsupported. Old/unreachable hosts fail closed. Feature-enabled custom hosts retain the existing opt-in/consent behavior.

Because releases remain feature-OFF, no zg-engine or new native libraries are bundled and no new bundled-library attribution payload is introduced. A future decision to ship the feature must include the upstream Apache-2.0 and native dependency licenses/notices, as required by `wp-zvec-handoff.md`; this decision does not waive that gate.

## Verification

- Default and `--features zvec` host capability unit test: 1 passed each.
- SearchSettings Vitest: 5 passed, covering present/absent capability, legacy/unreachable host, toggle, consent and build errors.
- `npx tsc --noEmit`: passed.
- Strict host clippy with `--no-deps`: passed. Strict protocol clippy remains blocked by pre-existing `control.rs` large-enum diagnostic; no change made in parallel-owned files.
- Release GNU host/CLI OFF and ON builds: passed. Musl OFF: passed; musl ON: upstream failure above.

Ownership exceptions: host command/protocol files only, needed for compiled capability reporting. No changes to parallel-owned wasm-host, autonomy, client, or failing-test files. No board writes.
