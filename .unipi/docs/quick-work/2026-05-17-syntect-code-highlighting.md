---
title: "Syntect Code Highlighting"
type: quick-work
date: 2026-05-17
---

# Syntect Code Highlighting

## Task
Replace the hand-written TUI code-block syntax highlighter with Syntect so more languages render with proper syntax colors.

## Changes
- `Cargo.toml`: added workspace dependencies for `syntect` and `syntect-assets`.
- `crates/oino-tui/Cargo.toml`: wired the TUI crate to the new dependencies.
- `crates/oino-tui/src/markdown.rs`: removed the manual keyword tokenizer and replaced it with a Syntect highlighter backed by bat syntax/theme assets. Code blocks now keep highlighting state across lines and resolve languages by token/extension/aliases.
- `README.md`: updated the Markdown rendering description to mention Syntect/bat-backed syntax coloring.

## Verification
- `cargo clippy --workspace --all-targets -- -D warnings`
- `cargo test --workspace`

## Notes
`syntect-assets` gives a broader bat-derived syntax set than Syntect's small built-in defaults, including languages such as TypeScript/TSX, TOML, YAML, Dockerfile, Kotlin, Swift, Vue, JSON, SQL, Go, Java, Ruby, PHP, Rust, Python, shell, HTML/XML, and CSS.
