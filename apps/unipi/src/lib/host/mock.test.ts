import { afterEach, describe, expect, it } from 'vitest';
import { z } from 'zod';
import { createMockHost, type MockScenario } from './mock';
import { appCall, hostCall, hostClient, setHostClientFactory } from './factory';
import { call } from '../ipc';
import { fsList, fsRead } from '../fs';
import { cronList } from '../cron';
import { forwardList } from '../forward';
import type { SessionEventDto, SessionSnapshot } from './client';
import rec from '../../dev/mock/fixtures/rec-math.json';

const scenario: MockScenario = {
  name: 'test',
  hosts: [
    {
      id: 'pc1',
      name: 'coffee',
      cards: [{ session_id: 's1', name: 'Math', cwd: '/home/u/proj', run_state: 'idle' }],
      fs: { '/home/u/proj': { kind: 'dir', children: { 'a.ts': { kind: 'file', text: 'export {}\n' }, src: { kind: 'dir', children: {} } } } },
      sessions: { [rec.snapshot.session_id]: rec as unknown as { snapshot: SessionSnapshot; events: Array<{ at: number; dto: SessionEventDto }> } },
      ops: { cron_list: () => ({ jobs: [], paused: true }) },
    },
    { id: 'pc2', name: 'desk', conn: { state: 'offline', message: 'No route', hint: 'Is unipi-host running?' } },
  ],
};

afterEach(() => setHostClientFactory(undefined));

describe('mock host (WP-A1)', () => {
  it('routes host ops to the right host and records them', async () => {
    const world = createMockHost(scenario);
    setHostClientFactory(world.client);
    const list = await fsList('pc1', '/home/u/proj');
    expect(list.ok && list.value.entries.map((e) => e.name).sort()).toEqual(['a.ts', 'src']);
    const read = await fsRead('pc1', '/home/u/proj/a.ts');
    expect(read.ok && read.value.text).toBe('export {}\n');
    const jobs = await cronList('pc1');
    expect(jobs.ok && jobs.value.paused).toBe(true); // scenario override wins over the built-in
    expect(world.log.map((l) => `${l.host}:${l.op}`)).toEqual(['pc1:fs_list', 'pc1:fs_read', 'pc1:cron_list']);
    expect(world.log[0].args).toEqual({ path: '/home/u/proj', hidden: undefined, outside: undefined });
  });

  it('answers app-level ops (paired hosts, forwards) and routes legacy call({hostId}) to the host', async () => {
    const world = createMockHost(scenario);
    setHostClientFactory(world.client);
    const hosts = await call('list_hosts', {}, z.array(z.object({ id: z.string(), name: z.string() }).passthrough()));
    expect(hosts.ok && hosts.value.map((h) => h.name)).toEqual(['coffee', 'desk']);
    expect((await forwardList()).ok).toBe(true);
    const legacy = await appCall('sessions_list', { hostId: 'pc1' }, z.array(z.object({ session_id: z.string() })));
    expect(legacy.ok && legacy.value[0].session_id).toBe('s1');
    expect(world.log.find((l) => l.op === 'sessions_list')?.host).toBe('pc1');
  });

  it('an offline host rejects with E_TRANSPORT and exposes its connection state', async () => {
    const world = createMockHost(scenario);
    setHostClientFactory(world.client);
    const r = await hostCall('pc2', 'fs_roots', {}, z.unknown());
    expect(r.ok ? undefined : r.error.code).toBe('E_TRANSPORT');
    expect(hostClient('pc2').conn()).toMatchObject({ state: 'offline', hint: 'Is unipi-host running?' });
    expect(hostClient('pc1').conn().state).toBe('connected');
    world.client('pc1').setConnection({ state: 'offline' });
    expect(hostClient('pc1').conn().state).toBe('offline');
  });

  it('unknown ops are answered with null and flagged unhandled in the log', async () => {
    const world = createMockHost(scenario);
    expect(await world.client('pc1').request('no_such_op', { x: 1 })).toBeNull();
    expect(world.log.at(-1)).toMatchObject({ host: 'pc1', op: 'no_such_op', unhandled: true });
  });

  it('replays a recorded session: snapshot first, then the script; commands are logged', async () => {
    const world = createMockHost(scenario);
    const seen: { snapshot?: SessionSnapshot; events: SessionEventDto[] } = { events: [] };
    const stream = world.client('pc1').openSession({ kind: 'open', session: rec.snapshot.session_id }, {
      snapshot: (s) => (seen.snapshot = s),
      events: (items) => seen.events.push(...items),
      closed: () => {},
    });
    await Promise.resolve();
    expect(seen.snapshot?.entries?.length).toBe(rec.snapshot.entries.length);
    expect(seen.events.map((e) => e.seq)).toEqual(rec.events.map((_, i) => i + 1));
    expect(seen.events[0].event.type).toBe('agent_start');
    await stream.cmd({ type: 'prompt', message: 'hi' });
    expect(world.log.map((l) => `${l.kind}:${l.op}`)).toEqual(['open:session_open', 'cmd:prompt']);
    stream.close();
  });

  it('a missing session closes the stream with E_NOT_FOUND', async () => {
    const world = createMockHost(scenario);
    let error: { code?: string } | undefined;
    world.client('pc1').openSession({ kind: 'open', session: 'nope' }, { snapshot: () => {}, events: () => {}, closed: (e) => (error = e) });
    await Promise.resolve();
    expect(error?.code).toBe('E_NOT_FOUND');
  });

  it('hub events reach openEvents subscribers until closed', () => {
    const world = createMockHost(scenario);
    const got: string[] = [];
    const sub = world.client('pc1').openEvents((e) => got.push(e.kind));
    world.emit({ kind: 'host_notice', level: 'info', text: 'hi' });
    sub.close();
    world.emit({ kind: 'host_notice', level: 'info', text: 'again' });
    expect(got).toEqual(['host_notice']);
  });
});

describe('mock generators + scenarios (WP-A1)', () => {
  it('genEntries is deterministic and chains parent ids', async () => {
    const { genEntries, genCards } = await import('../../dev/mock/gen');
    const a = genEntries({ seed: 3, turns: 4, thinking: true, diffs: true, compaction: true, error: true });
    expect(genEntries({ seed: 3, turns: 4, thinking: true, diffs: true, compaction: true, error: true })).toEqual(a);
    a.slice(1).forEach((e, i) => expect(e.parentId).toBe(a[i].id));
    expect(a.some((e) => e.type === 'compaction')).toBe(true);
    expect(genCards(5).map((c) => c.session_id)).toEqual(['s001', 's002', 's003', 's004', 's005']);
  });

  it('the A1 scenarios load into a world', async () => {
    const { SCENARIOS } = await import('../../dev/mock/scenarios');
    expect(Object.keys(SCENARIOS)).toEqual(expect.arrayContaining(['empty', 'no-host', 'offline']));
    const off = createMockHost(SCENARIOS.offline);
    expect(off.client('pc-coffee').conn().state).toBe('offline');
    const none = createMockHost(SCENARIOS['no-host']);
    expect(await none.client('app').request('list_hosts')).toEqual([]);
  });
});
