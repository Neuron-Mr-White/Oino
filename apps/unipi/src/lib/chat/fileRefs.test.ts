// @vitest-environment jsdom
import { describe, expect, it, vi } from 'vitest';
import { fileRefHits, fileRefLabel, hydrateFileRefs, splitByHits, unverifiedHit, verifyCandidates } from './fileRefs';
import { scanFileRefs } from './fileRefScan';
import { cachedStat, lookupPaths, pathStatUnavailable } from './pathStat';
import type { PathStatItem } from './store';

const CWD = '/home/u/proj';
/** A fake PC: these paths exist (absolute); relative asks resolve against CWD, bare names by suffix. */
function fakeChat(existing: Record<string, 'file' | 'dir'>) {
  const resolve = (p: string) => {
    const abs = p.startsWith('/') ? p : p.startsWith('~/') ? `/home/u/${p.slice(2)}` : `${CWD}/${p.replace(/^\.\//, '')}`;
    const clean = abs.replace(/\/+$/, '');
    if (existing[clean]) return { resolved: clean, kind: existing[clean]! };
    if (!p.startsWith('/') && !p.startsWith('~')) {
      const m = Object.keys(existing).filter((k) => k.endsWith(`/${p.replace(/\/+$/, '')}`));
      if (m.length === 1) return { resolved: m[0]!, kind: existing[m[0]!]! };
    }
    return { resolved: clean, kind: 'missing' as const };
  };
  const statPaths = vi.fn(async (paths: string[]): Promise<PathStatItem[]> => paths.map((path) => ({ path, ...resolve(path) })));
  return { statPaths, hostId: 'h1', pid: 1, cwd: () => CWD };
}

describe('verification (UNI-204)', () => {
  it('only existing paths become hits, resolved against the PC', async () => {
    const chat = fakeChat({ [`${CWD}/src/a.ts`]: 'file', [`${CWD}/apps/mobile/src/components/chat/Composer.tsx`]: 'file' });
    const hits = await fileRefHits(chat, 'see src/a.ts, src/gone.ts and Composer.tsx:12', 'prose');
    expect(hits.map((h) => [h.path, h.resolved, h.line])).toEqual([
      ['src/a.ts', `${CWD}/src/a.ts`, undefined],
      ['Composer.tsx', `${CWD}/apps/mobile/src/components/chat/Composer.tsx`, 12],
    ]);
  });

  it('batches every ask in a tick into one request and caches per chat', async () => {
    const chat = fakeChat({ [`${CWD}/a.ts`]: 'file' });
    await Promise.all([fileRefHits(chat, 'a.ts and b.ts', 'prose'), fileRefHits(chat, 'a.ts again, c/d.ts', 'prose')]);
    expect(chat.statPaths).toHaveBeenCalledTimes(1);
    expect(new Set(chat.statPaths.mock.calls[0]![0])).toEqual(new Set(['a.ts', 'b.ts', 'c/d.ts']));
    expect(cachedStat(chat, 'a.ts')?.kind).toBe('file');
    await fileRefHits(chat, 'a.ts', 'prose');
    expect(chat.statPaths).toHaveBeenCalledTimes(1); // served from cache
  });

  it('missing answers expire sooner than hits', () => {
    const chat = fakeChat({});
    const now = Date.now();
    return lookupPaths(chat, ['x.ts']).then(() => {
      expect(cachedStat(chat, 'x.ts', now + 1000)?.kind).toBe('missing');
      expect(cachedStat(chat, 'x.ts', now + 60_000)).toBeUndefined();
    });
  });

  it('a null answer (offline / older pi) keeps relative paths plain and is not cached', async () => {
    const chat = { statPaths: vi.fn(async () => null) };
    expect(await fileRefHits(chat, 'src/a.ts', 'prose')).toEqual([]);
    expect(cachedStat(chat, 'src/a.ts')).toBeUndefined();
    expect(pathStatUnavailable(chat)).toBe(true);
  });

  it('older pi: absolute paths fall back to unverified links (pre-UNI-204 behaviour)', async () => {
    const chat = { statPaths: vi.fn(async () => null) };
    const hits = await fileRefHits(chat, 'see /home/u/proj/src/a.ts:12 and /home/u/proj/docs/ and src/b.ts', 'prose');
    expect(hits.map((h) => [h.resolved, h.kind, h.line])).toEqual([
      ['/home/u/proj/src/a.ts', 'file', 12],
      ['/home/u/proj/docs', 'dir', undefined],
    ]);
  });

  it('the fallback ends as soon as the bridge answers again', async () => {
    let answer: PathStatItem[] | null = null;
    const chat = { statPaths: vi.fn(async (paths: string[]) => answer && paths.map((path) => ({ path, resolved: path, kind: 'missing' as const }))) };
    expect(await fileRefHits(chat, '/x/y.ts', 'prose')).toHaveLength(1);
    answer = [];
    expect(await fileRefHits(chat, '/x/z.ts', 'prose')).toEqual([]);
    expect(pathStatUnavailable(chat)).toBe(false);
  });

  it('unverifiedHit ignores relative candidates', () => {
    expect(unverifiedHit({ start: 0, end: 4, path: 'a.ts', explicit: false })).toBeUndefined();
  });

  it('tool output relative to the tool path: tries base-joined first, then as written', async () => {
    const chat = fakeChat({ [`${CWD}/packages/fusion/src/picker.ts`]: 'file' });
    const hits = await verifyCandidates(chat, scanFileRefs('picker.ts:15:  * ↑/↓ select', 'code'), 'packages/fusion/src');
    expect(hits[0]).toMatchObject({ resolved: `${CWD}/packages/fusion/src/picker.ts`, line: 15 });
  });

  it('directories come back as kind dir', async () => {
    const chat = fakeChat({ [`${CWD}/packages/dream`]: 'dir' });
    const hits = await fileRefHits(chat, '?? packages/dream/', 'code');
    expect(hits[0]).toMatchObject({ kind: 'dir', resolved: `${CWD}/packages/dream` });
  });
});

describe('splitByHits', () => {
  it('splits around hits keeping the original text', () => {
    const text = 'see src/a.ts:3 now';
    const parts = splitByHits(text, [{ start: 4, end: 14, path: 'src/a.ts', resolved: '/p/src/a.ts', kind: 'file', line: 3 }]);
    expect(parts.map((p) => (typeof p === 'string' ? p : `[${p.text}]`)).join('')).toBe('see [src/a.ts:3] now');
  });
});

describe('hydrateFileRefs (rendered markdown)', () => {
  const html = (s: string) => {
    const el = document.createElement('div');
    el.innerHTML = s;
    return el;
  };

  it('wraps verified paths in prose, leaves missing ones plain', async () => {
    const root = html('<p>edit src/a.ts and src/missing.ts then and/or stop</p>');
    await hydrateFileRefs(root, fakeChat({ [`${CWD}/src/a.ts`]: 'file' }));
    const refs = root.querySelectorAll<HTMLElement>('.file-ref');
    expect(refs).toHaveLength(1);
    expect(refs[0]!.textContent).toBe('src/a.ts');
    expect(refs[0]!.dataset.fileRef).toBe(`${CWD}/src/a.ts`);
    expect(root.textContent).toBe('edit src/a.ts and src/missing.ts then and/or stop');
  });

  it('inline code that is exactly a path becomes the link itself', async () => {
    const root = html('<p>see <code>Composer.tsx</code> and <code>npm test</code></p>');
    await hydrateFileRefs(root, fakeChat({ [`${CWD}/apps/Composer.tsx`]: 'file' }));
    const code = root.querySelector<HTMLElement>('code.file-ref-code')!;
    expect(code.textContent).toBe('Composer.tsx');
    expect(code.dataset.fileRef).toBe(`${CWD}/apps/Composer.tsx`);
    expect(root.querySelectorAll('code.file-ref-code')).toHaveLength(1);
  });

  it('paths inside fenced code (even split across highlight spans) are wrapped in place', async () => {
    const root = html('<pre class="cb"><code><span class="tok">$ cat </span><span class="s">src/a</span><span class="s">.ts:4</span>\nok</code></pre>');
    await hydrateFileRefs(root, fakeChat({ [`${CWD}/src/a.ts`]: 'file' }));
    const refs = [...root.querySelectorAll<HTMLElement>('.file-ref')];
    expect(refs.map((r) => r.textContent).join('')).toBe('src/a.ts:4');
    expect(refs.every((r) => r.dataset.fileLine === '4')).toBe(true);
    expect(root.querySelector('code')!.textContent).toBe('$ cat src/a.ts:4\nok');
  });

  it('markdown links to local files get the ref data; external links do not', async () => {
    const root = html('<p><a href="docs/plan.md#L3">the plan</a> <a href="https://x.dev/a.ts">web</a></p>');
    await hydrateFileRefs(root, fakeChat({ [`${CWD}/docs/plan.md`]: 'file' }));
    const local = root.querySelector<HTMLElement>('a[href="docs/plan.md#L3"]')!;
    expect(local.dataset.fileRef).toBe(`${CWD}/docs/plan.md`);
    expect(local.dataset.fileLine).toBe('3');
    expect(root.querySelector<HTMLElement>('a[href^="https"]')!.dataset.fileRef).toBeUndefined();
  });

  it('is idempotent and drops a stale pass', async () => {
    const chat = fakeChat({ [`${CWD}/src/a.ts`]: 'file' });
    const root = html('<p>src/a.ts</p>');
    await hydrateFileRefs(root, chat);
    await hydrateFileRefs(root, chat);
    expect(root.querySelectorAll('.file-ref')).toHaveLength(1);
    const stale = html('<p>src/a.ts</p>');
    await hydrateFileRefs(stale, chat, () => false);
    expect(stale.querySelector('.file-ref')).toBeNull();
  });

  it('does not touch task refs, code-block chrome or attachment chips', async () => {
    const root = html('<div class="cb-head"><span class="cb-lang">src/a.ts</span></div><p><a class="att-chip" href="#" data-att-path="/x/a.ts">a.ts</a></p>');
    await hydrateFileRefs(root, fakeChat({ [`${CWD}/src/a.ts`]: 'file', '/x/a.ts': 'file' }));
    expect(root.querySelector('.file-ref')).toBeNull();
  });
});

describe('fileRefLabel', () => {
  it('shows a short attachment label for blob-store paths', () => {
    expect(fileRefLabel('/home/u/.oino/host/blobs/deadbeefcafebabe0011223344556677')).toEqual({ label: 'attachment·deadbeef' });
  });
  it('splits basename and dimmed parent dir for a regular path', () => {
    expect(fileRefLabel('/home/u/proj/src/a.ts')).toEqual({ label: 'a.ts', dim: '/home/u/proj/src/' });
  });
  it('falls back to the whole path with no slash', () => {
    expect(fileRefLabel('a.ts')).toEqual({ label: 'a.ts' });
  });
});
