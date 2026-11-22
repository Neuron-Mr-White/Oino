// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';

const invoke = vi.fn();
vi.mock('@tauri-apps/api/core', () => ({ invoke: (...a: unknown[]) => invoke(...a) }));

import { readLocalFiles, registerDropZone, zoneAt } from './localFiles';

afterEach(() => {
  document.body.innerHTML = '';
  invoke.mockReset();
});

describe('native drop routing (UNI-256)', () => {
  function zone(el: HTMLElement | undefined, enabled = true) {
    return { el: () => el, enabled: () => enabled, hover: vi.fn(), drop: vi.fn() };
  }

  it('picks the innermost registered zone under the point; hidden / disabled zones never take it', () => {
    const outer = document.createElement('div');
    const inner = document.createElement('div');
    const leaf = document.createElement('span');
    inner.append(leaf);
    outer.append(inner);
    document.body.append(outer);
    const a = zone(outer);
    const b = zone(inner);
    const offA = registerDropZone(a);
    const offB = registerDropZone(b);
    (document as unknown as { elementFromPoint: (x: number, y: number) => Element | null }).elementFromPoint = () => leaf;
    expect(zoneAt(1, 1)).toBe(b);
    inner.hidden = true;
    expect(zoneAt(1, 1)).toBe(a);
    inner.hidden = false;
    offB();
    expect(zoneAt(1, 1)).toBe(a);
    const c = zone(inner, false);
    const offC = registerDropZone(c);
    expect(zoneAt(1, 1)).toBe(a);
    offA();
    offC();
    expect(zoneAt(1, 1)).toBeUndefined();
  });

  it('nothing under the point → no zone', () => {
    const off = registerDropZone(zone(document.body));
    (document as unknown as { elementFromPoint: () => null }).elementFromPoint = () => null;
    expect(zoneAt(5, 5)).toBeUndefined();
    off();
  });
});

describe('readLocalFiles', () => {
  it('reads bytes through the core and types the File by extension', async () => {
    invoke.mockImplementation(async (cmd: string, args: { path: string }) => {
      expect(cmd).toBe('local_file_bytes');
      return args.path.endsWith('.png') ? new Uint8Array([1, 2, 3]).buffer : [104, 105];
    });
    const files = await readLocalFiles([
      { path: '/tmp/shot.png', name: 'shot.png', size: 3, isDir: false },
      { path: '/tmp/n.txt', name: 'n.txt', size: 2, isDir: false },
    ]);
    expect(files.map((f) => [f.name, f.type, f.size])).toEqual([
      ['shot.png', 'image/png', 3],
      ['n.txt', 'text/plain', 2],
    ]);
  });
});
