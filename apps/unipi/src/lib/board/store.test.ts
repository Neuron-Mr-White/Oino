import { afterEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { createBoardStore } from './store';
import { setBoardTransport } from './transport';
import { fakeBoard, project, refused, task, type CliHandler } from './testing';

afterEach(() => setBoardTransport(undefined));

type World = {
  /** Per-slug task lists (the `list --all` answers). */
  boards: Record<string, { tasks: ReturnType<typeof task>[]; problems?: unknown[] }>;
  /** Overrides by handler key ('project list', 'list --all', 'status'). */
  overrides?: Record<string, CliHandler>;
  /** Handler outcomes by exact argv (writes like `move A done`). */
  exact?: Record<string, CliHandler>;
};

function setup(world: World) {
  const fake = fakeBoard({
    fallback: ({ args, project: slug }) => {
      const joined = args.join(' ');
      const exact = world.exact?.[joined];
      if (exact) {
        const outcome = exact({ args, project: slug, hostId: 'pc' });
        if (outcome !== undefined) return outcome;
      }
      if (joined === 'project list') {
        const override = world.overrides?.['project list'];
        if (override) return override({ args, project: slug, hostId: 'pc' });
        return [
          project({ slug: 'p1', name: 'One' }),
          project({ slug: 'p2', name: 'Two', archived: true }),
        ];
      }
      if (joined === 'list --all') {
        const board = world.boards[slug ?? ''] ?? { tasks: [], problems: [] };
        return { tasks: board.tasks, problems: board.problems ?? [] };
      }
      return refused(`no handler for ${joined}`, 'usage');
    },
  });
  setBoardTransport(fake.transport);
  const store = createRoot(() => createBoardStore());
  return { fake, store };
}

describe('board store', () => {
  it('opens a project: projects, rules, running and tasks', async () => {
    const { store } = setup({
      boards: {
        p1: { tasks: [task({ id: 'A', run: { session: 's' } }), task({ id: 'B' })], problems: [{ file: 'x.md', line: 1, error: 'bad' }] },
        p2: { tasks: [] },
      },
    });
    await store.open('pc', 'p1');
    expect(store.state.projects.map((p) => p.slug)).toEqual(['p1', 'p2']);
    expect(store.state.projects.find((p) => p.slug === 'p1')?.running).toBe(1);
    expect(store.state.rules.allowedMoves.todo).toEqual(['backlog', 'cancelled']);
    expect(store.state.running).toHaveLength(1);
    expect(store.state.running[0]!.task.id).toBe('A');
    expect(store.state.tasks.map((t) => t.id)).toEqual(['A', 'B']);
    expect(store.state.problems).toHaveLength(1);
    expect(store.state.loaded).toBe(true);
    expect(store.currentProject()?.name).toBe('One');
    expect(store.target()).toEqual({ hostId: 'pc', slug: 'p1' });
  });

  it('refresh() picks up changes; reload() only refetches tasks', async () => {
    const world: World = { boards: { p1: { tasks: [task({ id: 'A' })] }, p2: { tasks: [] } } };
    const { store, fake } = setup(world);
    await store.open('pc', 'p1');
    world.boards.p1 = { tasks: [task({ id: 'A' }), task({ id: 'C' })] };
    const before = fake.calls.length;
    await store.reload();
    expect(fake.calls.slice(before).map((call) => call.args.join(' '))).toEqual(['list --all']);
    expect(store.state.tasks.map((t) => t.id)).toEqual(['A', 'C']);
    world.boards.p1 = { tasks: [task({ id: 'C' })] };
    await store.refresh();
    expect(store.state.tasks.map((t) => t.id)).toEqual(['C']);
  });

  it('keeps the old board and records the error when a reload fails', async () => {
    let fail = false;
    const good = { tasks: [task({ id: 'A' })], problems: [] };
    const { store } = setup({
      boards: { p2: { tasks: [] } },
      exact: { 'list --all': () => (fail ? refused('disk on fire', 'io') : good) },
    });
    await store.open('pc', 'p1');
    expect(store.state.rules.final).toContain('done');
    fail = true;
    await store.reload();
    expect(store.state.error?.message).toBe('disk on fire');
    expect(store.state.tasks).toHaveLength(1);
    fail = false;
    await store.reload();
    expect(store.state.error).toBeUndefined();
  });

  it('drops a response for a project that is no longer open', async () => {
    const pending: Array<(value: unknown) => void> = [];
    const { store } = setup({
      boards: { p2: { tasks: [task({ id: 'NEW' })] } },
      exact: {
        'list --all': ({ project }) =>
          project === 'p1'
            ? new Promise((resolve) => {
                pending.push(resolve);
              })
            : undefined,
      },
    });
    const first = store.open('pc', 'p1');
    await new Promise((resolve) => setTimeout(resolve, 0));
    await store.open('pc', 'p2');
    pending.forEach((resolve) => resolve({ tasks: [task({ id: 'OLD' })], problems: [] }));
    await first;
    expect(store.state.tasks.map((t) => t.id)).toEqual(['NEW']);
  });

  it('applyTask upserts; optimistic() patches and rolls back', async () => {
    const { store } = setup({ boards: { p1: { tasks: [task({ id: 'A', status: 'todo', labels: ['x'] })] } } });
    await store.open('pc', 'p1');
    store.applyTask(task({ id: 'Z', status: 'backlog' }));
    expect(store.taskById('Z')?.status).toBe('backlog');
    store.applyTask({ ...store.taskById('A')!, title: 'Renamed' });
    expect(store.taskById('A')?.title).toBe('Renamed');
    const rollback = store.optimistic('A', { status: 'done', labels: [] });
    expect(store.taskById('A')?.status).toBe('done');
    rollback();
    expect(store.taskById('A')?.status).toBe('todo');
    expect(store.taskById('A')?.labels).toEqual(['x']);
    store.removeTask('Z');
    expect(store.taskById('Z')).toBeUndefined();
  });

  it('reports a projects error (PC reachable, board refusing)', async () => {
    const { store } = setup({
      boards: {},
      overrides: { 'project list': () => refused('bad token', 'rule') },
    });
    await store.open('pc', null);
    expect(store.state.projectsError?.message).toBe('bad token');
    expect(store.state.projectsLoaded).toBe(true);
  });
});
