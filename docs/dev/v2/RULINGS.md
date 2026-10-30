# v2 owner rulings (2026-10-31)

Owner: "take recommended for everything" — with these clarifications. Binding for all v2 work.

## ZCode audit ([zcode-audit.md](zcode-audit.md))
- Every Z item: the recommendation is accepted. For "ask" items, the option the Rec text recommends is taken.
- **Z-95 (override):** the git/status pane holds Git, long-horizon status, and todos/plans, plus a compact Running summary that links to the Session modal. The Session modal keeps Context / Running / Info (D-7) and is the only full Running view. The pane pushes the chat aside only at ≥ 1280 px; below that it shows as an overlay or capsule.
- **Z-43:** todos and plans go in that pane.
- **Z-61:** a paste of ≥ 5120 chars becomes a "Pasted text · N lines" chip and is expanded inline on send.
- **Z-65:** dictation is deferred.
- Gaps G-1..G-21 must all be closed: every unipi feature must be reachable from the UI.

## Spec questions — the defaults are accepted
- **core.md §7:** safe mode keeps providers on. One trap disables that extension for the session; the 3-in-10-min process breaker stays as a backstop. Subagents hibernate after 120 s idle. The hub evicts idle sessions after 900 s.
- **ext-abi.md §11:** honored Retry-After waits have a 1 h cumulative cap. `oino.browser` is renamed `unipi.browser`.
- **host.md §14:** tray label "UniPi (oino)". Archive = move to `~/.oino/trash/`, restorable from the archived list (Z-13).
- **browser.md §14:** if the hidden window fails the canaries, fall back to headless with a warning. Ask before the first CloakBrowser download. Canaries run nightly, plus a non-blocking advisory run on PRs that touch `crates/oino-browser/**`.
- **dist.md §15:** first version 2.0.0, with a new `oino-updater` minisign key. Public GitHub Releases are assumed, but changing repo visibility and publishing still need explicit owner approval at release time.
- **ui-protocol.md §16:** a third-party extension gets at most one sidebar page. The `accent` token maps to `claw`. The file-summary extension id is `unipi.file-summary`.

## Execution
- Waves, each ending green (build, tests, lints, QA) before the next:
  1. Core + extension ABI
  2. Host + app
  3. Browser, terminal and installers
  4. Full QA on every platform
- Windows builds run on `coffee`, macOS on `desmondgiam`, Android on a plugged-in phone. The owner makes those hosts and the phone available when a wave needs them.
