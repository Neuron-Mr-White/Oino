# Deviations: tui-client

Owner: `tui-client` (`crates/oino-tui`, `crates/oino-view`). Settled rows (#1–#4, #8–#14, with #5–#7
superseded by frontend-cmds T1–T6) are folded into
[modes.md](../modes.md#appendix-settled-deviations-folded-at-100-alpha2) (1.0.0-alpha.2).

## Open

- #4 / T7: inline images (kitty graphics via a post-draw escape pass).
- `LocalHost` writes core settings through its own `SettingsManager`; route them through
  `_oino/set_setting` so the running session sees them.
- `--remote` (needs `oino serve` and `_oino/list_sessions`, `_oino/get_keybindings`, `_oino/render`).
- `SessionTreeNode` `Deserialize` (the view parses `get_tree` JSON itself).
