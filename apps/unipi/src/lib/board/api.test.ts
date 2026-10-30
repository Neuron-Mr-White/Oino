/**
 * api over the CLI transport (v2): every endpoint maps to the argv the
 * `unipi-kanboard` CLI expects (ext-abi §5.5), refusals decode from the
 * `{ok:false, error, kind}` line, and the daemon-era server computations
 * (project summaries, dashboard) derive from `list --all` payloads.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { api, ApiError, describe as describeError, deriveDashboard, enteredStatus, filePath, parseError, projectSummaryOf, resetFileUrlCache, USER_RULES, type Task } from './api';
import { fakeBoard, ok, refused, task } from './testing';
import { setBoardTransport } from './transport';

const P = { slug: 'demo-a1b2', name: 'Demo' };

afterEach(() => {
  setBoardTransport(undefined);
  resetFileUrlCache();
});

describe('parseError', () => {
  it('reads the CLI failure line: rule kind, needsComment from the message', () => {
    const error = parseError({ ok: false, error: 'in_progress → todo requires --comment (why the stale run is being released)', kind: 'rule' });
    expect(error).toBeInstanceOf(ApiError);
    expect(error.status).toBe(409);
    expect(error.needsComment).toBe(true);
    expect(error.needsAgent).toBe(false);
    expect(error.message).toContain('requires --comment');
  });

  it('maps usage / not_found kinds and agent-only refusals', () => {
    expect(parseError({ ok: false, error: 'unexpected argument', kind: 'usage' }).status).toBe(400);
    expect(parseError({ ok: false, error: 'no project registered', kind: 'not_found' }).status).toBe(404);
    const agent = parseError({ ok: false, error: 'todo → in_progress is agent/system only: the agent that `start`ed it moves it on', kind: 'rule' });
    expect(agent.needsAgent).toBe(true);
    expect(parseError({ ok: false, error: '', kind: 'io' }).message).toBe('the board refused the change');
  });
});

describe('api over the CLI transport', () => {
  it('sends the subcommand argv with the project pinned', async () => {
    const fake = fakeBoard({ handlers: { 'list --all': () => ({ tasks: [task({ id: 'D-1' })], problems: [] }) } });
    setBoardTransport(fake.transport);
    const payload = await api.tasks('h', P.slug);
    expect(payload.tasks[0]!.id).toBe('D-1');
    expect(fake.calls).toEqual([{ hostId: 'h', args: ['list', '--all'], project: P.slug }]);
  });

  it('maps writes to their flags: create, move, note, edit, link, order, duplicate', async () => {
    const fake = fakeBoard({ fallback: () => task({ id: 'D-1' }) });
    setBoardTransport(fake.transport);
    await api.create('h', P.slug, { title: 'Fix the parser', body: 'notes', status: 'todo', priority: 'high', after: ['D-0'], labels: ['b'] });
    await api.move('h', P.slug, 'D-1', 'in_review', 'summary');
    await api.note('h', P.slug, 'D-1', 'a note');
    await api.edit('h', P.slug, 'D-1', { title: 'T', body: 'B', priority: 'low', labels: ['x'] });
    await api.link('h', P.slug, 'D-1', 'D-0');
    await api.unlink('h', P.slug, 'D-1', 'D-0');
    await api.order('h', P.slug, 'D-1', { before: 'D-2' });
    await api.order('h', P.slug, 'D-1', { top: true });
    await api.duplicate('h', P.slug, 'D-1');
    expect(fake.sent('add').at(-1)!.args).toEqual([
      'add', 'Fix the parser', '--body=notes', '--status', 'todo', '--priority', 'high', '--after', 'D-0', '--label', 'b', '--new-label',
    ]);
    expect(fake.sent('move D-1 in_review').at(-1)!.args).toEqual(['move', 'D-1', 'in_review', '--comment', 'summary']);
    expect(fake.sent('note D-1').at(-1)!.args).toEqual(['note', 'D-1', '--', 'a note']);
    expect(fake.sent('edit D-1').at(-1)!.args).toEqual(['edit', 'D-1', '--title=T', '--body=B', '--priority', 'low', '--labels=x', '--new-label']);
    expect(fake.sent('link D-1').at(-1)!.args).toEqual(['link', 'D-1', '--after', 'D-0']);
    expect(fake.sent('unlink D-1').at(-1)!.args).toEqual(['unlink', 'D-1', '--after', 'D-0']);
    expect(fake.sent('order D-1').at(0)!.args).toEqual(['order', 'D-1', '--before', 'D-2']);
    expect(fake.sent('order D-1').at(1)!.args).toEqual(['order', 'D-1', '--top']);
    expect(fake.sent('duplicate D-1').at(-1)!.args).toEqual(['duplicate', 'D-1']);
  });

  it('throws ApiError with the refusal flags on a CLI failure', async () => {
    const fake = fakeBoard({ handlers: { 'move D-1 blocked': () => refused('in_progress → blocked requires --comment (what you need)', 'rule') } });
    setBoardTransport(fake.transport);
    const error = await api.move('h', P.slug, 'D-1', 'blocked').catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).needsComment).toBe(true);
    expect(describeError(error)).toContain('requires --comment');
  });

  it('turns a host failure into ApiError status 0', async () => {
    const fake = fakeBoard({});
    setBoardTransport({
      ...fake.transport,
      async cli() {
        throw new Error('the PC is unreachable');
      },
    });
    const error = await api.tasks('h', P.slug).catch((e: unknown) => e);
    expect(error).toBeInstanceOf(ApiError);
    expect((error as ApiError).status).toBe(0);
  });

  it('health remembers the board home (attachment paths resolve against it)', async () => {
    const fake = fakeBoard({ handlers: { status: () => ({ ok: true, home: '/home/me/.oino/state/unipi.kanboard' }) } });
    setBoardTransport(fake.transport);
    await api.health('h');
    expect(fake.sent('status')[0]!.args).toEqual(['status']);
    // fileUrl downloads through the host blob channel — stubbed at the client seam in live tests.
    resetFileUrlCache();
  });

  it('attachment refs resolve to board-relative paths', () => {
    expect(filePath(P.slug, 'D-1', '1a2b3c4d-shot.png')).toBe('projects/demo-a1b2/attachments/D-1/1a2b3c4d-shot.png');
  });
});

describe('derived board data (the daemon-era server computations)', () => {
  const meta = { slug: 'p', name: 'Project' };

  it('projectSummaryOf counts lanes, running and the newest change', () => {
    const tasks = [
      task({ id: 'A', status: 'todo', updated: '2026-10-01T10:00:00Z' }),
      task({ id: 'B', status: 'todo', run: { session: 's' }, updated: '2026-10-03T10:00:00Z' }),
      task({ id: 'C', status: 'in_review', updated: '2026-10-02T10:00:00Z' }),
    ];
    const summary = projectSummaryOf(meta, tasks, []);
    expect(summary.total).toBe(3);
    expect(summary.counts).toMatchObject({ todo: 2, in_review: 1, backlog: 0 });
    expect(summary.running).toBe(1);
    expect(summary.updatedAt).toBe('2026-10-03T10:00:00Z');
  });

  it('enteredStatus reads the write-path vocabulary', () => {
    expect(enteredStatus('finished: shipped it')).toBe('in_review');
    expect(enteredStatus('blocked: need an API key')).toBe('blocked');
    expect(enteredStatus('unblocked')).toBe('todo');
    expect(enteredStatus('rework: shorter')).toBe('todo');
    expect(enteredStatus('cancelled')).toBe('cancelled');
    expect(enteredStatus('moved todo → done (bulk)')).toBe('done');
    expect(enteredStatus('undo: done → todo (state restored)')).toBe('todo');
    expect(enteredStatus('created in backlog')).toBe('backlog');
    expect(enteredStatus('a plain note')).toBeNull();
  });

  it('deriveDashboard builds inbox/upNext/activity/throughput like the daemon', () => {
    const now = Date.parse('2026-10-07T12:00:00Z');
    const tasks: Task[] = [
      task({
        id: 'R1',
        status: 'in_review',
        updated: '2026-10-07T09:00:00Z',
        created: '2026-10-01T09:00:00Z',
        activity: [
          { at: '2026-10-06T09:00:00Z', actor: 'agent', session: 's1', text: 'started (session s1)' },
          { at: '2026-10-07T09:00:00Z', actor: 'agent', session: 's1', text: 'finished: shipped the relay' },
        ],
      }),
      task({
        id: 'B1',
        status: 'blocked',
        updated: '2026-10-07T08:00:00Z',
        created: '2026-10-01T08:00:00Z',
        activity: [{ at: '2026-10-07T08:00:00Z', actor: 'agent', text: 'blocked: need an API key' }],
      }),
      task({ id: 'T1', status: 'todo', ready: true, updated: '2026-10-05T08:00:00Z', created: '2026-10-02T08:00:00Z', priority: 'high', activity: [] }),
      task({ id: 'D1', status: 'done', updated: '2026-10-06T10:00:00Z', activity: [{ at: '2026-10-06T10:00:00Z', actor: 'user', text: 'moved in_review → done' }] }),
    ];
    const data = deriveDashboard({ projects: [meta], perProject: [{ meta, tasks, problems: [] }], running: [] }, now);
    expect(data.inbox.map((item) => item.task.id)).toEqual(['B1', 'R1']);
    expect(data.inbox[1]!.excerpt).toBe('shipped the relay');
    expect(data.inbox[0]!.excerpt).toBe('need an API key');
    expect(data.inbox[0]!.waitingSince).toBe('2026-10-07T08:00:00Z');
    expect(data.upNext.map((item) => item.task.id)).toEqual(['T1']);
    expect(data.readyTotal).toBe(1);
    expect(data.projects[0]).toMatchObject({ slug: 'p', review: 1, blocked: 1, ready: 1, problems: 0 });
    expect(data.doneAt).toEqual(['2026-10-06T10:00:00.000Z']);
    expect(data.activity[0]!.text).toContain('finished:');
    // A completed review cycle: in → out (undo-aware stamps come from activity).
    expect(data.reviewWaits).toHaveLength(0);
  });
});

describe('USER_RULES mirror', () => {
  it('matches the user rows of the transition table', async () => {
    expect(USER_RULES.allowedMoves.todo).toEqual(['backlog', 'cancelled']);
    expect(USER_RULES.allowedMoves.in_review).toEqual(['done', 'todo', 'backlog', 'archived']);
    expect(USER_RULES.allowedMoves.archived).toEqual([]);
    expect(USER_RULES.commentRequired.in_progress?.todo).toContain('stale run');
    await expect(api.rules('h')).resolves.toMatchObject({ statuses: USER_RULES.statuses });
  });
});
