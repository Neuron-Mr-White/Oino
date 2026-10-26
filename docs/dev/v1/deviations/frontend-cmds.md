# Deviations: frontend-cmds

The command table, rows #1–#15 and T1–T6 are folded into
[modes.md](../modes.md#appendix-settled-deviations-folded-at-100-alpha2) (1.0.0-alpha.2).

## Open

- T7: TUI inline images are `[image: <mime>]` placeholders (kitty / iTerm / sixel not done).
- `oino serve` / `--remote` (see deviations/modes.md).
- `models.classify` / `generate-image`; usage of `models.stream` is not recorded.
- OAuth sign-in verified end to end only with the faux provider.
- The desktop and mobile apps now use `oino_client::FrontendApi` (design-desktop, design-mobile);
  `_oino/service_call` has no native GPUI consumer yet.
