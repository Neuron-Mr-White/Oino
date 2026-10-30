/**
 * Session control centre (UNI-160) store logic: hello.fusion/work, the
 * `fusion`/`work`/`stats`/`info` pushes, set_fusion/watch/work_* request
 * plumbing — same fake-transport pattern as store.test.ts.
 */
import { afterEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { createChat } from './store';
import { setChatTransport, type ChatConnection, type ChatTransport } from '../session/link';

type Sent = { msg: Record<string, unknown> };

function fakeTransport() {
  const sent: Sent[] = [];
  let onLine: ((line: string) => void) | undefined;
  const conn: ChatConnection = {
    send: async (line) => {
      sent.push({ msg: JSON.parse(line) });
    },
    close: () => {},
  };
  const transport: ChatTransport = {
    list: async () => ({ sessions: [] }),
    open: (_host, _pid, onLineCb, onState) => {
      onLine = onLineCb;
      queueMicrotask(() => onState('open'));
      return conn;
    },
  };
  return {
    sent,
    push(msg: Record<string, unknown>) {
      onLine?.(JSON.stringify(msg));
    },
    reply(t: string, reply: Record<string, unknown>) {
      const last = [...sent].reverse().find((s) => s.msg.t === t);
      if (!last) throw new Error(`nothing sent with t=${t}`);
      onLine?.(JSON.stringify({ ref: last.msg.ref, ...reply }));
    },
    transport,
  };
}

const helloBase = { t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] };

afterEach(() => setChatTransport(undefined));

describe('control centre store (UNI-160)', () => {
  it('hello.fusion / hello.work land in state', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({
          ...helloBase,
          fusion: { leads: [{ key: 'p/l', name: 'Lead' }], sidekicks: [], default: {}, effort: {}, active: undefined },
          work: [{ id: 'bg-1', kind: 'bg', title: 'build', dot: 'running', startedAt: 1, canStop: true, canRerun: false, canBackground: false }],
        });
        await Promise.resolve();
        expect(chat.state.fusion?.leads).toEqual([{ key: 'p/l', name: 'Lead' }]);
        expect(chat.state.work).toHaveLength(1);
        expect(chat.state.work[0]!.dot).toBe('running');
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
  });

  it('a fusion push updates fusionStatus, including clearing it to undefined', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push(helloBase);
        await Promise.resolve();
        f.push({ t: 'fusion', status: { leadName: 'Lead', leadEffort: 'high', sidekickName: 'Side', sidekickEffort: 'low', busy: true } });
        await Promise.resolve();
        expect(chat.state.fusionStatus?.leadName).toBe('Lead');
        expect(chat.state.fusionStatus?.busy).toBe(true);
        f.push({ t: 'fusion', status: undefined });
        await Promise.resolve();
        expect(chat.state.fusionStatus).toBeUndefined();
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
  });

  it('a work push replaces the work list', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push(helloBase);
        await Promise.resolve();
        f.push({ t: 'work', items: [{ id: 'agent-1', kind: 'subagent', title: 'explorer', dot: 'failed', startedAt: 1, canStop: false, canRerun: true, canBackground: false }] });
        await Promise.resolve();
        expect(chat.state.work).toHaveLength(1);
        expect(chat.state.work[0]!.id).toBe('agent-1');
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
  });

  it('setFusionSingle / setFusionPair send set_fusion and resolve on ack', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let singleOk = false;
    let pairOk = false;
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push(helloBase);
        await Promise.resolve();
        const p1 = chat.setFusionSingle('p/m', 'high');
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'set_fusion', single: 'p/m', effort: 'high' });
        f.reply('set_fusion', { t: 'ack' });
        await p1;
        singleOk = true;

        const p2 = chat.setFusionPair('p/lead', 'p/side', 'high', 'low');
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'set_fusion', lead: 'p/lead', sidekick: 'p/side', leadEffort: 'high', sidekickEffort: 'low' });
        f.reply('set_fusion', { t: 'ack' });
        await p2;
        pairOk = true;
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    expect(singleOk).toBe(true);
    expect(pairOk).toBe(true);
    dispose();
  });

  it('watch() sends the flags as given, not a request/reply', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push(helloBase);
        await Promise.resolve();
        chat.watch({ stats: true });
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'watch', stats: true });
        chat.watch({ info: true });
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'watch', info: true });
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
  });

  it('a stats push lands in state.stats; an info push lands in state.infoGroups', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push(helloBase);
        await Promise.resolve();
        f.push({ t: 'stats', stats: { tokensIn: 10, tokensOut: 20, cacheHit: 5, cost: 0.02, tps: 30 } });
        await Promise.resolve();
        expect(chat.state.stats?.tps).toBe(30);
        f.push({ t: 'info', groups: [{ id: 'session', label: 'Session', stats: [{ label: 'Turns', value: '3' }] }] });
        await Promise.resolve();
        expect(chat.state.infoGroups[0]!.label).toBe('Session');
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
  });

  it('workLog resolves the page from the matching work_log reply, and an error reply too', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let page1: unknown;
    let page2: unknown;
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push(helloBase);
        await Promise.resolve();
        const p1 = chat.workLog('bg-1');
        await Promise.resolve();
        f.reply('work_log', { t: 'work_log', id: 'bg-1', text: 'line one\nline two', more: false });
        page1 = await p1;

        const p2 = chat.workLog('bg-2');
        await Promise.resolve();
        f.reply('work_log', { t: 'error', message: 'Unknown background task ID: bg-2' });
        page2 = await p2;
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    expect(page1).toEqual({ text: 'line one\nline two', more: false });
    expect(page2).toEqual({ error: 'Unknown background task ID: bg-2' });
    dispose();
  });

  it('workStop / workRerun / workBackground resolve on ack and reject with the error message otherwise', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let stopped = false;
    let rejectedMessage: string | undefined;
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push(helloBase);
        await Promise.resolve();
        const p1 = chat.workStop('bg-1');
        await Promise.resolve();
        f.reply('work_stop', { t: 'ack' });
        await p1;
        stopped = true;

        const p2 = chat.workBackground('sidekick');
        await Promise.resolve();
        f.reply('work_background', { t: 'error', message: "This item can't be stopped." });
        try {
          await p2;
        } catch (error) {
          rejectedMessage = (error as Error).message;
        }
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    expect(stopped).toBe(true);
    expect(rejectedMessage).toBe("This item can't be stopped.");
    dispose();
  });

  it('UNI-282: workTranscript / graphOutput / graphHistory resolve their replies; "unknown type" = needs a newer unipi', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const got: Record<string, unknown> = {};
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push(helloBase);
        await Promise.resolve();
        const p1 = chat.workTranscript('op-1');
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'work_transcript', id: 'op-1' });
        f.reply('work_transcript', { t: 'work_transcript', id: 'op-1', items: [{ kind: 'task', text: 'do it' }, null, { kind: 'tool', name: 'bash', arg: 'ls', output: 'a', isError: false, running: false }], report: 'done' });
        got.transcript = await p1;

        const p2 = chat.graphOutput('scan', 'g-1');
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'lh_graph', op: 'output', id: 'scan', graphId: 'g-1' });
        f.reply('lh_graph', { t: 'lh_graph', op: 'output', output: { graphId: 'g-1', id: 'scan', status: 'completed', instruction: 'Scan', text: 'full' } });
        got.output = await p2;

        const p3 = chat.graphHistory();
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'lh_graph', op: 'history' });
        f.reply('lh_graph', { t: 'lh_graph', op: 'history', runs: [{ mode: 'graph', title: 'old', status: 'done', items: [], counts: { total: 0, done: 0, running: 0, failed: 0, queued: 0 } }, { junk: true }] });
        got.history = await p3;

        const p4 = chat.graphOutput('x');
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg.graphId).toBeUndefined();
        f.reply('lh_graph', { t: 'error', message: 'unknown type "lh_graph"' });
        got.old = await p4;

        const p5 = chat.workTranscript('op-2');
        await Promise.resolve();
        f.reply('work_transcript', { t: 'error', message: 'No such subagent: op-2' });
        got.missing = await p5;
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    expect(got.transcript).toEqual({ id: 'op-1', items: [{ kind: 'task', text: 'do it' }, { kind: 'tool', name: 'bash', arg: 'ls', output: 'a', isError: false, running: false }], report: 'done' });
    expect(got.output).toEqual({ graphId: 'g-1', id: 'scan', status: 'completed', instruction: 'Scan', text: 'full' });
    expect((got.history as unknown[]).length).toBe(1);
    expect(got.old).toEqual({ error: 'Reading a step’s result needs a newer unipi on the PC.', unsupported: true });
    expect(got.missing).toEqual({ error: 'No such subagent: op-2' });
    dispose();
  });

  it('UNI-282: graphStop resolves on ack; an older bridge rejects with "needs a newer unipi"', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let stopped = false;
    let message: string | undefined;
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push(helloBase);
        await Promise.resolve();
        const p1 = chat.graphStop();
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'lh_graph', op: 'stop' });
        f.reply('lh_graph', { t: 'ack' });
        await p1;
        stopped = true;
        const p2 = chat.graphStop();
        await Promise.resolve();
        f.reply('lh_graph', { t: 'error', message: 'unknown type' });
        try {
          await p2;
        } catch (error) {
          message = (error as Error).message;
        }
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    expect(stopped).toBe(true);
    expect(message).toBe('Stopping the graph from the app needs a newer unipi on the PC.');
    dispose();
  });
});
