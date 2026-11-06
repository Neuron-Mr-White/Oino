# v3 worker rules (every implementation package)

Repo `/home/oi/Projects/Personal/llm/oino`, branch `v2`. You implement ONE work package of the Oino v3 round.
Read first: the package's row in the **Work packages** and **Acceptance tests** sections of `docs/dev/v3/01-spec.md` (the spec; implement exactly your package, stay inside its owned files plus its declared ordered edits), the decision sections it references in that spec, and `docs/dev/v3/00-owner-decisions.md` (binding; wins any conflict). Background/context ticket (read-only, do NOT write to the board): `/usr/share/oino/extensions/unipi.kanboard/bin/unipi-kanboard --actor agent --project oino-f6a9c2 show <ticket id>` — where the ticket and spec disagree, the spec wins. References: `docs/dev/v3/ref-droid-prompts.md`, `docs/dev/v3/ref-droid-compaction-context.md`, `docs/dev/v3/ref-dsh-superagent.md`.
Rules:
- The working tree is shared with other workers running in parallel. Never `git stash`, `checkout --`, `reset`, `clean`, never revert or reformat files you did not change, stage only your own paths/hunks (no `git add -A/-u`, no repo-wide `cargo fmt`; format only your files). Commit early and often with prefix `[v3/<package key>]`.
- If a build breaks in a file you do not own (another worker mid-edit), wait a few minutes and retry; do not fix it. If it stays broken >20 min, report it.
- Builds: root workspace `cargo test -p <crate>` (CARGO_BUILD_JOBS=8; run `scripts/target-gc.sh` if target/ > 30 GB); `extensions/` is a separate wasm workspace (`cargo xtask wasm`; commit `extensions/dist` only for packages you changed); `apps/unipi/src-tauri` is standalone; frontend in `apps/unipi`: `npx tsc --noEmit` + scoped `npx vitest run <files>`. Run scoped tests, not the whole world. `cargo clippy -p <crate> -- -D warnings` on crates you changed.
- Old sessions must still replay; keep the cached prefix byte-identical (spec §C). No features beyond the spec; if the spec is genuinely ambiguous, pick the most conservative reading and list it in your report.
Done = every acceptance test for your package exists and passes, clippy clean on touched crates, all work committed.
Report: commits, files touched, tests run + results, deviations from the spec, notes for downstream packages.

## Ownership exceptions (supervisor, 2026-11-07)
- File ownership in the spec is for sequencing, not a reason to stop. If your package needs a change in a file outside your owned list, make the narrowest change that does the job and list it under "Ownership exceptions" in your report. Do not return "blocked" for ownership reasons.
- The only limit: do not edit a file that a package running at the same time as yours owns (concurrent pairs: wp-exec-bg ∥ fix-tools-acp ∥ fix-cache-legacy; wp-autonomy-ui ∥ sa-backend; wp-compaction ∥ wp-zvec ∥ sa-ui). If you must, keep it to a few lines and say so in the report.
- Upstream packages listed as your inputs are committed before you start. If one is still missing something you need, implement the minimum yourself rather than stopping.
- The gpt-sol audit `r-qa-backend` (P1-1 … P1-10, P2) is the bar: any finding in your package's area must be fixed with a regression test that reproduces the audit's exact input.
