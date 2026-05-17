---
title: "Input Placeholder File Attach Hint"
type: quick-work
date: 2026-05-17
---

# Input Placeholder File Attach Hint

## Task
Update the composer input placeholder so users know `@` attaches/searches file paths.

## Changes
- `crates/oino-tui/src/composer.rs`: changed `INPUT_PLACEHOLDER` to `Ask Oino • / commands • @ file paths`.

## Verification
```bash
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

All passed.

## Notes
- Existing render tests cover the placeholder through `INPUT_PLACEHOLDER`.
