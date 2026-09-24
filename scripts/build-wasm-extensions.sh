#!/usr/bin/env bash
# Build the test WASM extension components (wasm32-wasip2 produces real
# components via wasm-component-ld) and copy them to their consumers.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --target wasm32-wasip2 --manifest-path extensions/Cargo.toml \
    -p fixture -p fixture-provider -p provider-openai-compat -p provider-anthropic -p provider-openai-responses -p ext-budget -p ext-presets -p ext-search -p ext-ralph -p ext-vcc -p ext-notify
target="extensions/target/wasm32-wasip2/release"
mkdir -p crates/oino-wasm-host/tests/fixtures examples/extensions/rust-wasm-fixture
cp "$target/fixture.wasm" crates/oino-wasm-host/tests/fixtures/fixture.wasm
cp "$target/fixture.wasm" examples/extensions/rust-wasm-fixture/plugin.wasm
cp "$target/fixture_provider.wasm" crates/oino-wasm-host/tests/fixtures/fixture_provider.wasm
mkdir -p crates/oino-app/assets/providers
cp "$target/provider_openai_compat.wasm" crates/oino-app/assets/providers/openai_compat.wasm
cp "$target/provider_anthropic.wasm" crates/oino-app/assets/providers/anthropic.wasm
cp "$target/provider_openai_responses.wasm" crates/oino-app/assets/providers/openai_responses.wasm
# Built-in extension packages embed their component as `plugin.wasm`.
cp "$target/ext_budget.wasm" extensions/built-in/budget/extensions/budget/plugin.wasm
cp "$target/ext_presets.wasm" extensions/built-in/presets/extensions/presets/plugin.wasm
cp "$target/ext_search.wasm" extensions/built-in/search/extensions/search/plugin.wasm
cp "$target/ext_ralph.wasm" extensions/built-in/ralph-loop/extensions/ralph-loop/plugin.wasm
cp "$target/ext_vcc.wasm" extensions/built-in/vcc/extensions/vcc/plugin.wasm
cp "$target/ext_notify.wasm" extensions/built-in/notify/extensions/notify/plugin.wasm
