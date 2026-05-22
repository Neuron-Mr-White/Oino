# Oino theme system docs

These docs explain Oino's first-class theme pipeline for contributors and extension authors. For day-to-day theme selection, see the [Themes section in the extension user guide](../extension-kernel/user-guide.md#themes).

## Current implementation

Oino currently supports:

- built-in themes: `system`, `oino-dark`, `oino-light`, `oino-mono`, and `oino-aurora`;
- JSON theme files under `~/.oino/themes/**/*.json` and `<project>/.oino/themes/**/*.json`;
- extension theme contributions declared from installed packages;
- `/theme` and `/settings theme` for previewing and saving project/global themes;
- component-role rendering across the app shell, transcript, markdown/code, settings, suggestions, extension surfaces, badges, and diagnostics.

Implemented theme picker controls are:

| Key | Action |
|---|---|
| `Enter` | Preview the selected theme without saving. |
| `p` | Save the selected theme as the project theme. |
| `g` | Save the selected theme as the global theme. |
| `r` | Reset the project theme to inherit global. |
| `R` | Reset the global theme to `system`. |
| `Esc` / `←` | Cancel preview or return/close. |

## Read next

- [Theme schema](theme-schema.md) — current JSON contract, token roles, alias compatibility, and validation behavior.
- [Theme precedence and UX](theme-precedence-and-ux.md) — project/global precedence, picker behavior, and remaining follow-ups.
- [Component classification](component-classification.md) — how TUI text/components map to theme roles.
- [Generated text inventory](text-inventory.generated.md) — generated source-string inventory used for classification.
- [DeepSeek TUI research](deepseek-tui-research.md) — historical inspiration notes for the first theme pass.
