import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { MuxTreeResultSchema } from './schemas';

const fixtures: { name: string; schema: string; value: unknown; ok: boolean }[] = JSON.parse(
  readFileSync('../../protocol/fixtures/m2.json', 'utf8'),
);
const treeFixtures = fixtures.filter((fixture) => fixture.schema === 'TreeData' && fixture.ok);
const invalidTree = fixtures.find((fixture) => fixture.schema === 'TreeData' && !fixture.ok);

describe('MuxTreeResultSchema', () => {
  it('parses every valid tree fixture with a path', () => {
    for (const fixture of treeFixtures) {
      const result = MuxTreeResultSchema.safeParse({ tree: fixture.value, path: 'lan' });
      expect(result.success, `${fixture.name}: ${JSON.stringify(result.error?.issues)}`).toBe(true);
    }
  });

  it('rejects an invalid tree and an invalid path', () => {
    expect(invalidTree).toBeDefined();
    const result = MuxTreeResultSchema.safeParse({ tree: invalidTree!.value, path: 'lan' });
    expect(result.success).toBe(false);
    expect(MuxTreeResultSchema.safeParse({ tree: treeFixtures[0].value, path: 'hyperloop' }).success).toBe(false);
  });
});
