#!/usr/bin/env bash
# Build the test WASM extension components (wasm32-wasip2 produces real
# components via wasm-component-ld) and copy them to their consumers.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --target wasm32-wasip2 --manifest-path extensions/Cargo.toml \
    -p fixture -p fixture-provider -p provider-openai-compat
target="extensions/target/wasm32-wasip2/release"
mkdir -p crates/oino-wasm-host/tests/fixtures examples/extensions/rust-wasm-fixture
cp "$target/fixture.wasm" crates/oino-wasm-host/tests/fixtures/fixture.wasm
cp "$target/fixture.wasm" examples/extensions/rust-wasm-fixture/plugin.wasm
cp "$target/fixture_provider.wasm" crates/oino-wasm-host/tests/fixtures/fixture_provider.wasm
mkdir -p crates/oino-app/assets/providers
cp "$target/provider_openai_compat.wasm" crates/oino-app/assets/providers/openai_compat.wasm
