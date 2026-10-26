# Deviations: gpui-desktop

Settled rows are folded into [gpui-app.md](../gpui-app.md#appendix-settled-deviations-folded-at-100-alpha2)
(G1–G5, G10, G11; 1.0.0-alpha.2). The UX spec is [design.md](../design.md); the parity checklist and
perf numbers are in `apps/oino-desktop/DESIGN-STATUS.md`, the review and its resolution in
[design/review-desktop.md](../design/review-desktop.md).

## Open

| Item | Notes |
|---|---|
| Perf | 4-pane whole-run p99 9.3–14.9 ms vs the 10 ms budget (design.md §2.11). |
| Keyboard | StatusPanel and Queue key tables, DiffViewer `u`, tooltips on keyboard focus. |
| Review P2 | "Thought for Ns", model-menu submenus / pinned group, secret reveal toggle, info overlay vertical tabs, DiffViewer line numbers / per-file actions / split view, compact-now confirmation, updater-driven update badge, skeleton loaders, exit animations, run-label shimmer. |
| Core requests | File checkpoints (Undo / rewind), single-item queue commands, `disabledExtensions` in the core loader, `_oino/topics`, `_oino/agents`. |
| Platforms | Windows rendering not checked (locked console); macOS rendering not checked over SSH; no signing / notarisation. |
