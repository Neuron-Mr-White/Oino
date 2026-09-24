# pi oracle fixtures

`gen.ts` drives pi v0.87.1's provider stream functions (`streamSimple`) with an
injected `fetch` that captures the request (url, headers, JSON body) and serves
a canned SSE response, then records pi's normalized `AssistantMessageEvent`s.

Regenerate:

```bash
cd /tmp/pi-upstream            # pi v0.87.1 checkout
npm install --ignore-scripts   # never run lifecycle scripts
npm --prefix packages/ai run generate-models   # needs network (models.dev)
OINO_REPO=/path/to/oino PI_DIR=/tmp/pi-upstream \
    node --experimental-strip-types scripts/pi-oracle/gen.ts
```

Output: `extensions/crates/{provider-anthropic,provider-openai-responses}/fixtures/pi/*.json`
(`context`, `sse`, `status`, `model`, `request`, `events`). The oino-app
`pi_oracle_*` tests replay these against the bundled WASM components and
assert byte-equal request bodies, header parity and 1:1 event parity.
