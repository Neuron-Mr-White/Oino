import { afterEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { createBoardStore } from './store';
import { setBoardTransport } from './transport';
import { fakeBoard, project, task, type FakeBoardOptions } from './testing';

afterEach(() => setBoardTransport(undefined));

function setup(routes: FakeBoardOptions['routes']) {
  const fake = fakeBoard({ routes });
  setBoardTransport(fake.transport);
  const store = createRoot(() => createBoardStore());
  return { fake, store };
}

const projects = [project({ slug: 'p1', name: 'One' }), project({ slug: 'p2', name: 'Two', archived: true })];

describe('board store', () => {
  it('opens a project: projects, rules, running and tasks', async () => {
    const { store } = setup({
      'GET /api/projects': () => ({ json: projects }),
      'GET /api/rules': () => ({ json: { statuses: [], allowedMoves: { todo: ['backlog'] }, commentRequired: {}, final: [] } }),
      'GET /api/running': () => ({ json: { running: [{ slug: 'p1', project: 'One', task: task({ id: 'A', run: { session: 's' } }) }] } }),
      'GET /api/projects/p1/tasks': () => ({ json: { tasks: [task({ id: 'A' }), task({ id: 'B' })], problems: [{ file: 'x.md', line: 1, error: 'bad' }] } }),
    });
    await store.open('pc', 'p1');
    expect(store.state.projects.map((p) => p.slug)).toEqual(['p1', 'p2']);
    expect(store.state.rules.allowedMoves.todo).toEqual(['backlog']);
    expect(store.state.running).toHaveLength(1);
    expect(store.state.tasks.map((t) => t.id)).toEqual(['A', 'B']);
    expect(store.state.problems).toHaveLength(1);
    expect(store.state.loaded).toBe(true);
    expect(store.currentProject()?.name).toBe('One');
    expect(store.target()).toEqual({ hostId: 'pc', slug: 'p1' });
  });

  it('refresh() picks up changes; reload() only refetches tasks', async () => {
    let tasks = [task({ id: 'A' })];
    const { store, fake } = setup({
      'GET /api/projects': () => ({ json: projects }),
      'GET /api/rules': () => ({ json: { statuses: [], allowedMoves: {}, commentRequired: {}, final: [] } }),
      'GET /api/running': () => ({ json: { running: [] } }),
      'GET /api/projects/p1/tasks': () => ({ json: { tasks, problems: [] } }),
    });
    await store.open('pc', 'p1');
    tasks = [task({ id: 'A' }), task({ id: 'C' })];
    const before = fake.requests.length;
    await store.reload();
    expect(fake.requests.slice(before).map((r) => r.path)).toEqual(['/api/projects/p1/tasks']);
    expect(store.state.tasks.map((t) => t.id)).toEqual(['A', 'C']);
    tasks = [task({ id: 'C' })];
    await store.refresh();
    expect(store.state.tasks.map((t) => t.id)).toEqual(['C']);
  });

  it('keeps the old board and records the error when a reload fails', async () => {
    let fail = false;
    const { store } = setup({
      'GET /api/projects': () => ({ json: projects }),
      'GET /api/rules': () => ({ status: 404, json: { error: 'old daemon' } }),
      'GET /api/running': () => ({ status: 404, json: {} }),
      'GET /api/projects/p1/tasks': () => (fail ? { status: 500, json: { error: 'disk on fire' } } : { json: { tasks: [task({ id: 'A' })], problems: [] } }),
    });
    await store.open('pc', 'p1');
    expect(store.state.rules.final).toContain('done'); // permissive fallback
    fail = true;
    await store.reload();
    expect(store.state.error?.message).toBe('disk on fire');
    expect(store.state.tasks).toHaveLength(1);
    fail = false;
    await store.reload();
    expect(store.state.error).toBeUndefined();
  });

  it('drops a response for a project that is no longer open', async () => {
    let release: () => void = () => {};
    const { store } = setup({
      'GET /api/projects': () => ({ json: projects }),
      'GET /api/rules': () => ({ json: { statuses: [], allowedMoves: {}, commentRequired: {}, final: [] } }),
      'GET /api/running': () => ({ json: { running: [] } }),
      'GET /api/projects/p1/tasks': () => new Promise((resolve) => (release = () => resolve({ json: { tasks: [task({ id: 'OLD' })], problems: [] } }))),
      'GET /api/projects/p2/tasks': () => ({ json: { tasks: [task({ id: 'NEW' })], problems: [] } }),
    });
    const first = store.open('pc', 'p1');
    await new Promise((resolve) => setTimeout(resolve, 0));
    await store.open('pc', 'p2');
    release();
    await first;
    expect(store.state.tasks.map((t) => t.id)).toEqual(['NEW']);
  });

  it('applyTask upserts; optimistic() patches and rolls back', async () => {
    const { store } = setup({
      'GET /api/projects': () => ({ json: projects }),
      'GET /api/rules': () => ({ json: { statuses: [], allowedMoves: {}, commentRequired: {}, final: [] } }),
      'GET /api/running': () => ({ json: { running: [] } }),
      'GET /api/projects/p1/tasks': () => ({ json: { tasks: [task({ id: 'A', status: 'todo', labels: ['x'] })], problems: [] } }),
    });
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

  it('reports a projects error (PC reachable, daemon refusing)', async () => {
    const { store } = setup({
      'GET /api/projects': () => ({ status: 401, json: { error: 'bad token' } }),
      'GET /api/rules': () => ({ json: { statuses: [], allowedMoves: {}, commentRequired: {}, final: [] } }),
      'GET /api/running': () => ({ json: { running: [] } }),
    });
    await store.open('pc', null);
    expect(store.state.projectsError?.status).toBe(401);
    expect(store.state.projectsLoaded).toBe(true);
  });
});
