// @vitest-environment node
import { describe, expect, it } from 'vitest';
import { readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { POLL_ALLOWLIST } from './poll-allowlist';

const root = fileURLToPath(new URL('../../', import.meta.url));
function files(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => e.isDirectory() ? files(`${dir}/${e.name}`) : [`${dir}/${e.name}`]);
}
describe('polling regression gate', () => {
  it('allows no new interval or timeout call sites outside the reviewed inventory', () => {
    const failures: string[] = [];
    for (const file of files(root)) {
      const name = file.slice(root.length).replace(/^\//, '');
      if (!/\.tsx?$/.test(name) || /\.(test|bench)\./.test(name) || name.startsWith('dev/')) continue;
      if (name === 'lib/topics/poll-allowlist.ts') continue;
      const text = readFileSync(file, 'utf8');
      const actual = { intervals: (text.match(/\bsetInterval\s*\(/g) ?? []).length,
        timeouts: (text.match(/\bsetTimeout\s*\(/g) ?? []).length };
      const allowed = POLL_ALLOWLIST[name] ?? { intervals: 0, timeouts: 0 };
      if (actual.intervals > allowed.intervals || actual.timeouts > allowed.timeouts) failures.push(`${name}: ${JSON.stringify(actual)}`);
    }
    expect(failures, 'Move new polling into the reviewed topics fallback or document an explicit budget').toEqual([]);
  });
});
