/**
 * Turn model budgets (app spec WP-A2 acceptance, §3.6), vitest 5 benchmarks:
 * - 2 000-turn full build ≤ 25 ms;
 * - tail update (a streaming delta on a 2 000-turn session) ≤ 0.5 ms.
 *
 *   npx vitest bench --run src/lib/session/turns.bench.ts
 */
import { expect, test } from 'vitest';
import { genEntries } from '../../dev/mock/gen';
import { buildTurns, createTurnsIndex, type TurnsInput } from './turns';
import type { SessionEntry } from './types';

const entries = genEntries({ seed: 11, turns: 2000, toolsPerTurn: 2, thinking: true, diffs: true, steers: true, compaction: true }) as unknown as SessionEntry[];
const base: TurnsInput = { entries, leafId: entries.at(-1)!.id, entriesRev: 1, liveTools: {}, run: { running: false } };

/** Median latency (ms) from a vitest/tinybench result. */
function median(result: unknown): number {
  const r = result as { latency?: { p50?: number; mean?: number }; p50?: number; mean?: number };
  return r.latency?.p50 ?? r.latency?.mean ?? r.p50 ?? r.mean ?? Number.NaN;
}

test('2 000-turn full build ≤ 25 ms', async ({ bench }) => {
  expect(buildTurns(base).filter((t) => t.kind === 'turn')).toHaveLength(2000);
  const result = await bench('full build (2 000 turns)', () => {
    buildTurns(base);
  }).run();
  console.info(`full build p50 ${median(result).toFixed(2)} ms`);
  expect(median(result)).toBeLessThanOrEqual(25);
});

test('tail update on a 2 000-turn session ≤ 0.5 ms', async ({ bench }) => {
  const index = createTurnsIndex();
  index.update(base);
  let n = 0;
  const result = await bench('tail update (streaming delta)', () => {
    n += 1;
    index.update({ ...base, run: { running: true, startedAt: 0 }, streaming: { startedAt: 0, message: { role: 'assistant', content: [{ type: 'thinking', thinking: 'x'.repeat(n % 400) }, { type: 'text', text: `streamed ${n}` }] } } });
  }).run();
  expect(index.lastMode).toBe('tail');
  console.info(`tail update p50 ${median(result).toFixed(3)} ms`);
  expect(median(result)).toBeLessThanOrEqual(0.5);
});
