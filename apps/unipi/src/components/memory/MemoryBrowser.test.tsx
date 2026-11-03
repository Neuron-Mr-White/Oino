// @vitest-environment jsdom
/**
 * Settings › Memory › Browse memories (owner fix 2026-11-03): the app modal over
 * `memory_browse` — a page of 50 from a 7,000-memory store, Load more, host-side
 * search on input, the project scope, skeleton / empty / error + Retry states.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { createMockHost, type MockMemory, type MockOp } from '../../dev/mock/host';
import { closeMemoryBrowser, MemoryBrowser, MemoryBrowserLayer, openMemoryBrowser } from './MemoryBrowser';

const HOST = 'this-pc';
const BASE = Date.UTC(2026, 10, 3);

function store(n: number): MockMemory[] {
  const projects = ['oino', 'tally', 'cloud_agent'];
  return Array.from({ length: n }, (_, i) => ({
    id: `m${i}`,
    title: i === 4321 ? 'Redis cache decision' : `Memory ${i}`,
    kind: 'decision',
    project: projects[i % 3]!,
    updated_at: BASE - i * 60_000,
    body: i === 4321 ? 'Prefer refresh tokens.' : `Body ${i}`,
  }));
}

function boot(opts: { memories?: MockMemory[]; ops?: Record<string, MockOp>; project?: string; delay?: number } = {}) {
  setDesktopOverride(true);
  const world = createMockHost({
    name: 't',
    hosts: [{ id: HOST, name: 'This computer', transport: 'local', conn: { state: 'connected' }, memories: opts.memories ?? store(7000), memoryDelayMs: opts.delay, ops: opts.ops }],
  });
  setHostClientFactory(world.client);
  render(() => <MemoryBrowser target={{ hostId: HOST, ...(opts.project ? { project: opts.project } : {}) }} onClose={() => {}} />);
  return world;
}

const rows = () => document.querySelectorAll('[data-testid="memory-row"]');
const calls = (world: ReturnType<typeof boot>) => world.log.filter((l) => l.op === 'memory_browse').map((l) => l.args as Record<string, unknown>);

afterEach(() => {
  cleanup();
  closeMemoryBrowser();
  setHostClientFactory(undefined);
  resetHubs();
  resetLayout();
  setDesktopOverride(undefined);
});

describe('MemoryBrowser', () => {
  it('is a titled modal with a close button; shows a skeleton, then the newest 50 of 7,000', async () => {
    const world = boot({ delay: 30 });
    const dialog = screen.getByTestId('memory-browser');
    expect(dialog.getAttribute('role')).toBe('dialog');
    expect(dialog.getAttribute('aria-label')).toBe('Memories');
    expect(screen.getByTestId('model-picker-close')).toBeTruthy();
    expect(screen.getByTestId('memory-skeleton')).toBeTruthy();
    await waitFor(() => expect(rows().length).toBe(50));
    expect(rows()[0]!.textContent).toContain('Memory 0');
    expect(screen.getByTestId('memory-count').textContent).toBe('7,000 memories in All projects · newest first');
    expect(calls(world)[0]).toEqual({ limit: 50 });
    // the scope lists the store's projects with counts
    const scope = screen.getByTestId('memory-scope') as HTMLSelectElement;
    await waitFor(() => expect([...scope.options].map((o) => o.textContent)).toEqual(['All projects', 'oino (2,334)', 'cloud_agent (2,333)', 'tally (2,333)']));
    fireEvent.click(screen.getByTestId('memory-more'));
    await waitFor(() => expect(rows().length).toBe(100));
    expect(calls(world).at(-1)).toEqual({ limit: 50, cursor: '50' });
  });

  it('searches on the host as the user types (debounced) and opens a memory', async () => {
    const world = boot();
    await waitFor(() => expect(rows().length).toBe(50));
    fireEvent.input(screen.getByTestId('memory-search'), { target: { value: 'refresh tokens' } });
    await waitFor(() => expect(rows().length).toBe(1));
    expect(calls(world).at(-1)).toEqual({ query: 'refresh tokens', limit: 50 });
    expect(screen.getByTestId('memory-count').textContent).toBe('1 match in All projects');
    fireEvent.click(rows()[0]!);
    expect(screen.getByTestId('memory-detail').textContent).toContain('Redis cache decision');
    fireEvent.click(screen.getByTestId('memory-back'));
    expect(rows().length).toBe(1);
    fireEvent.input(screen.getByTestId('memory-search'), { target: { value: 'nothing like this' } });
    await waitFor(() => expect(screen.getByTestId('memory-empty').textContent).toContain('No memory in All projects matches “nothing like this”'));
  });

  it('scopes to one project (and back to All projects)', async () => {
    const world = boot({ project: 'oino' });
    await waitFor(() => expect(rows().length).toBe(50));
    expect(calls(world)[0]).toEqual({ project: 'oino', limit: 50 });
    expect(screen.getByTestId('memory-count').textContent).toBe('2,334 memories in oino · newest first');
    fireEvent.change(screen.getByTestId('memory-scope'), { target: { value: '' } });
    await waitFor(() => expect(screen.getByTestId('memory-count').textContent).toBe('7,000 memories in All projects · newest first'));
  });

  it('an empty store says so; a failure shows the error with Retry', async () => {
    boot({ memories: [] });
    await waitFor(() => expect(screen.getByTestId('memory-empty').textContent).toContain('Nothing remembered on this PC yet'));
    cleanup();
    let fail = true;
    const world = boot({
      ops: {
        memory_browse: () => {
          if (fail) throw Object.assign(new Error('disk on fire'), { code: 'E_INTERNAL' });
          return { items: [{ id: 'a', title: 'Back again', kind: 'pattern', project: 'oino', updated_at: BASE, body: 'x' }], total: 1, projects: [], store: true };
        },
      },
    });
    await waitFor(() => expect(screen.getByTestId('memory-error').textContent).toContain('disk on fire'));
    fail = false;
    fireEvent.click(screen.getByTestId('memory-retry'));
    await waitFor(() => expect(rows().length).toBe(1));
    expect(calls(world).length).toBe(2);
  });

  it('the shell layer opens and closes it', async () => {
    setDesktopOverride(true);
    setHostClientFactory(createMockHost({ name: 't', hosts: [{ id: HOST, name: 'h', conn: { state: 'connected' }, memories: store(3) }] }).client);
    render(() => <MemoryBrowserLayer />);
    expect(screen.queryByTestId('memory-browser')).toBeNull();
    openMemoryBrowser({ hostId: HOST });
    await waitFor(() => expect(rows().length).toBe(3));
    fireEvent.click(screen.getByTestId('model-picker-close'));
    await waitFor(() => expect(screen.queryByTestId('memory-browser')).toBeNull());
  });
});
