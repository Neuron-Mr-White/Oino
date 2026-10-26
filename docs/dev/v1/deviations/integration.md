# Deviations: integration

Owner: `integration` (P4/P5 of the v1.alpha plan; `integration-2` for the follow-up round).
Rows #1–#4 and #6–#8 of 1.0.0-alpha.1 are folded into
[extension-api.md](../extension-api.md#appendix-settled-deviations-folded-at-100-alpha2), #5 into
[runtime.md](../runtime.md#appendix-settled-deviations-folded-at-100-alpha2), #10–#11 and the
alpha.2 changes into [architecture.md](../architecture.md#appendix-settled-deviations-folded-at-100-alpha2);
#9 (ACP `--model`) was resolved by frontend-cmds #12, #12 (version alpha.1) is superseded by N4.

## 1.0.0-alpha.2 (integration-2)

| # | Change | Reason |
|---|---|---|
| I2-1 | Settled rows of every `deviations/*.md` (and `extensions/unipi/utility/DEVIATIONS.md`) moved into an appendix of the spec they amend; the deviation files keep only open items (README §5 rule 5). `gpui-app.md` §5 points to `design.md`. | P5 fold. |
| I2-2 | Version `1.0.0-alpha.2` everywhere (architecture appendix N4); every component rebuilt with rustc 1.99.0 and `extensions/dist/checksums.json` regenerated (also picks up the settings store in the components core-settings did not rebuild). | Release bump. |
| I2-3 | Root crates switched to `[lints] workspace = true`, root `thiserror = "2"` (N3; closes core-types-session W2 and kanboard RK4). | quality §2. |
| I2-4 | User docs (`docs/*.md`) updated for package manager, `oino update`, `/login`, providers-more, desktop features, `unipi.utility`, TUI regular mode, ACP `--model`. | Docs matched alpha.1. |

## Open

- `cargo xtask layering`, `cargo deny`, golden e2e suite `tests/e2e/` + `PARITY.md`, criterion
  benches (quality §§1, 5, 7).
- Release: alpha.2 artifacts, signing, tag (STATUS §7–§8).
