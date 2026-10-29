import { afterEach, describe, expect, it, vi } from 'vitest';

const projects = vi.fn();
vi.mock('./api', () => ({ api: { projects: (hostId: string) => projects(hostId) } }));

import {
  clearBoardJump,
  hydrateTaskRefs,
  knownPrefixes,
  pendingBoardJump,
  pickProjectForCwd,
  pickProjectForRef,
  refPrefix,
  resetPrefixCache,
  splitTaskRefs,
} from './portal';
import { addTaskFromChat, openTaskFromChat } from '../chat/boardPortal';
import { hasMarkedReturnToChat, clearReturnToChat } from '../chat/open';
import type { ProjectSummary } from './api';

const P = (slug: string, prefix: string, root?: string, archived = false): ProjectSummary => ({ slug, name: slug, prefix, root, archived });

afterEach(() => {
  vi.clearAllMocks();
  resetPrefixCache();
  clearBoardJump();
  clearReturnToChat();
});

describe('task ref matching (UNI-181)', () => {
  it('refPrefix reads exact task ids only', () => {
    expect(refPrefix('UNI-172')).toBe('UNI');
    expect(refPrefix('UNI-172x')).toBeUndefined();
    expect(refPrefix('uni-172')).toBeUndefined();
  });

  it('splitTaskRefs keeps only known prefixes (UTF-8 / SHA-256 stay text)', () => {
    const parts = splitTaskRefs('Fixed UNI-172 and UNI-17, not UTF-8 or SHA-256.', new Set(['UNI']));
    expect(parts).toEqual(['Fixed ', { ref: 'UNI-172' }, ' and ', { ref: 'UNI-17' }, ', not UTF-8 or SHA-256.']);
    expect(splitTaskRefs('nothing here', new Set(['UNI']))).toEqual(['nothing here']);
  });

  it('hydrateTaskRefs wraps prose refs and exact inline-code refs, skipping code blocks and links', () => {
    const root = document.createElement('div');
    root.innerHTML =
      '<p>See UNI-172 now.</p><p><code>UNI-181</code> and <code>UNI-1 x</code></p>' +
      '<pre><code>UNI-999</code></pre><p><a href="https://x">UNI-5</a></p>';
    hydrateTaskRefs(root, new Set(['UNI']));
    const refs = [...root.querySelectorAll<HTMLElement>('[data-task]')].map((el) => el.dataset.task);
    expect(refs).toEqual(['UNI-172', 'UNI-181']);
    // Idempotent.
    hydrateTaskRefs(root, new Set(['UNI']));
    expect(root.querySelectorAll('[data-task]')).toHaveLength(2);
  });
});

describe('project resolution (UNI-181)', () => {
  const list = [
    P('unipi-faa347', 'UNI', '/home/oi/Projects/unipi'),
    P('heartpick-427ef7', 'HEA', '/home/oi/Projects/heartpick'),
    P('heartpick-merchant-24102c', 'HEA', '/home/oi/Projects/heartpick/merchant'),
    P('old-uni', 'UNI', '/old', true),
  ];

  it('a single live project with the prefix wins (archived ones never match)', () => {
    expect(pickProjectForRef(list, 'UNI-172')).toBe('unipi-faa347');
    expect(pickProjectForRef(list, 'ZZZ-1')).toBeUndefined();
  });

  it('a shared prefix is tie-broken by the chat cwd (deepest root), then the last opened', () => {
    expect(pickProjectForRef(list, 'HEA-3', '/home/oi/Projects/heartpick/merchant/src')).toBe('heartpick-merchant-24102c');
    expect(pickProjectForRef(list, 'HEA-3', '/home/oi/Projects/heartpick/web')).toBe('heartpick-427ef7');
    expect(pickProjectForRef(list, 'HEA-3', '/tmp', 'heartpick-merchant-24102c')).toBe('heartpick-merchant-24102c');
  });

  it('pickProjectForCwd uses the root containing the cwd, else the last opened', () => {
    expect(pickProjectForCwd(list, '/home/oi/Projects/unipi/unipi-app')).toBe('unipi-faa347');
    expect(pickProjectForCwd(list, '/tmp', 'heartpick-427ef7')).toBe('heartpick-427ef7');
    expect(pickProjectForCwd(list, '/tmp')).toBeUndefined();
  });
});

describe('known prefixes (UNI-181)', () => {
  it('caches per PC and skips archived projects', async () => {
    projects.mockResolvedValue([P('a', 'UNI'), P('b', 'OLD', undefined, true)]);
    expect([...(await knownPrefixes('h1'))]).toEqual(['UNI']);
    await knownPrefixes('h1');
    expect(projects).toHaveBeenCalledTimes(1);
  });

  it('a failed fetch yields no prefixes and retries next time', async () => {
    projects.mockRejectedValueOnce(new Error('board down')).mockResolvedValueOnce([P('a', 'UNI')]);
    expect((await knownPrefixes('h2')).size).toBe(0);
    expect([...(await knownPrefixes('h2'))]).toEqual(['UNI']);
  });
});

describe('chat → board jumps (UNI-181)', () => {
  it('openTaskFromChat hands the board a jump, marks the return and switches tab', () => {
    openTaskFromChat('h1', 'UNI-172', '/p');
    expect(pendingBoardJump()).toEqual({ hostId: 'h1', taskId: 'UNI-172', cwd: '/p', fromChat: true });
    expect(hasMarkedReturnToChat()).toBe(true);
    expect(location.hash).toBe('#/board');
  });

  it('addTaskFromChat asks for the New task sheet', () => {
    addTaskFromChat('h1', '/p');
    expect(pendingBoardJump()).toEqual({ hostId: 'h1', newTask: true, cwd: '/p', fromChat: true });
  });
});
