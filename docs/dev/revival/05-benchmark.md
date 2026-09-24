# P5: FrontierHarness Evaluation

Goal: measure Oino on the same basis as the published field and use the result to pick
defaults. Source: `github.com/runta-dev/frontier-harness-eval` ("Evaluate your own
harness" kit, `skills/frontierharness-eval`).

## Prerequisites

- P4.0 headless mode (`oino -p --json`).
- P3 providers, so Oino can reach Fireworks/Kimi K3 through Chat Completions (or the
  gateway's Responses or Messages endpoint).
- The benchmark's gateway speaks OpenAI Responses and Anthropic Messages; configure
  Oino's Responses or Anthropic component against it.

## Rules (from the benchmark design)

- **Never run the formal 30 tasks during development.** Implicit prefix caches stay warm
  for hours and bias results. Use the kit's smoke task set for all debugging.
- One attempt per task, fresh environment per run.
- Report what the field reports: pass rate, median cost per pass, median cost per task
  (failures included), token-weighted and median-cell cache hit rate, median turns,
  median wall time.

## Configurations to run

1. `standard` preset, no caps (comparable to "Pi").
2. `minimal` preset.
3. `standard` + `oino.budget` with `max_turns` near the field median turns for passing
   runs.

## Interpretation

- Treat pass-rate differences under ~10 points as noise (n=30, one attempt).
- Rank changes by cost per pass and token-weighted cache hit. The target is to match or
  beat Pi's $2.43 median cost per pass while staying within noise of its 60% pass rate.
- For any run with cache hit below 90%, use the P4.2 fingerprint diff to find the
  divergence before changing prompts.

## Deliverable

`docs/dev/revival/results-<date>.md` with the table above, per-task notes on the most
expensive runs, and the decided defaults for preset and caps.
