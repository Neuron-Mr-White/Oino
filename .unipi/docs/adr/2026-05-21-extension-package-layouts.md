# ADR: Oino Extension Package Layouts

Status: accepted
Date: 2026-05-21

## Decision

Oino extension discovery uses only Oino-owned `.oino` paths. It does not infer extensions from Pi, Claude, AGENTS, or other agent conventions unless a future import command explicitly copies or generates Oino manifests under these roots.

## Standard roots

Global user roots:

- `~/.oino/extensions/` — global local extension manifests.
- `~/.oino/extension-packages/` — globally installed package directories.
- `~/.oino/extension-registry/` — local/fixture registry package metadata.
- `~/.oino/extension-state/` — global extension-owned persistent state.

Project roots:

- `<project>/.oino/extensions/` — project local extension manifests.
- `<project>/.oino/extension-packages/` — project installed package directories.
- `<project>/.oino/wasm-extensions/` — project WASM extension bundles.
- `<project>/.oino/session-extensions/` — session-scoped extension manifests.
- `<project>/.oino/dev/extensions/` — development extension manifests.
- `<project>/.oino/extension-state/` — project/session extension-owned persistent state.
- `<project>/.oino/extension-assets/` — package assets copied or materialized by package lifecycle flows.

## Package contents

`oino.package.json` can reference:

- Extension manifests (`extensions[].manifest`).
- Resources such as skills, prompts, themes, system prompts, project instructions, and assets (`resources[]`).
- Asset files (`assets[]`).
- Example projects/snippets (`examples[]`).
- Documentation files (`docs[]`).

A package is valid when it contributes at least one extension, resource, asset, example, or doc entry.

## Source precedence

Discovery keeps the existing deterministic source precedence:

1. Built-ins.
2. Global sources.
3. Project sources.
4. Session sources.
5. Development sources.

Within a scope, source kind and path ordering are deterministic. Missing roots are ignored without diagnostics.

## Consequences

- Oino can safely coexist with other agent ecosystems without accidentally loading their files.
- Package install/update/remove logic can target stable roots in later tasks.
- Extension persistence has explicit global/project/session storage roots and can be cleaned up during uninstall.
