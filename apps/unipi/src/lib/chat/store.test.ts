/**
 * The store's handling of session-navigation requests (UNI-158): sessions{},
 * tree{}, session_new/resume/fork, tree_go, session_rename — request/response
 * plumbing over a fake transport (no real socket, no Solid root needed since
 * createStore works standalone).
 */
import { afterEach, describe, expect, it } from 'vitest';
import { createRoot } from 'solid-js';
import { applyMessage, createChat, BusyError, customInsertIndex, reconcileBtwPages, treeGoRestoreOf, type BtwPage, type TreeGoRestore } from './store';
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
    transport,
    sent,
    /** Delivers one bridge line to the chat. */
    push(msg: Record<string, unknown>) {
      onLine?.(JSON.stringify(msg));
    },
    /** Replies `ack`/an error for the last sent message with this `t`. */
    reply(t: string, reply: Record<string, unknown>) {
      const last = [...sent].reverse().find((s) => s.msg.t === t);
      if (!last) throw new Error(`nothing sent with t=${t}`);
      onLine?.(JSON.stringify({ ref: last.msg.ref, ...reply }));
    },
  };
}

afterEach(() => setChatTransport(undefined));

describe('chat store — session navigation (UNI-158)', () => {
  it('sessions{} resolves items and more from the matching reply', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();
        const promise = chat.sessions('cwd', 'bug');
        await Promise.resolve();
        f.reply('sessions', { t: 'sessions', items: [{ path: '/a.jsonl', id: 'a', cwd: '/p', firstMessage: 'fix bug', modified: 1, messageCount: 2, current: false }], more: false });
        const result = await promise;
        expect(result.items).toHaveLength(1);
        expect(result.items[0]!.path).toBe('/a.jsonl');
        expect(result.more).toBe(false);
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
  });

  it('tree{} resolves nodes from the matching reply', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let result: unknown;
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();
        const promise = chat.tree();
        await Promise.resolve();
        f.reply('tree', { t: 'tree', nodes: [{ id: 'm1', parentId: null, kind: 'user', preview: 'hi', onPath: true, current: true }] });
        result = await promise;
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    expect((result as { id: string }[])[0]!.id).toBe('m1');
    dispose();
  });


  it('sessionNew resolves on ack', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let ok = false;
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();
        const promise = chat.sessionNew();
        await Promise.resolve();
        f.reply('session_new', { t: 'ack' });
        await promise;
        ok = true;
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    expect(ok).toBe(true);
    dispose();
  });

  it('tree_go rejects with BusyError on code:"busy"; force:true is sent through on retry', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let caught: unknown;
    let forcedSent = false;
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();
        const promise = chat.treeGo('m1');
        await Promise.resolve();
        f.reply('tree_go', { t: 'error', code: 'busy', message: 'pi is busy' });
        try {
          await promise;
        } catch (error) {
          caught = error;
        }
        const retry = chat.treeGo('m1', false, true);
        await Promise.resolve();
        forcedSent = f.sent.some((s) => s.msg.t === 'tree_go' && s.msg.force === true);
        f.reply('tree_go', { t: 'ack' });
        await retry;
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    expect(caught).toBeInstanceOf(BusyError);
    expect(forcedSent).toBe(true);
    dispose();
  });

  it('renameSession sends session_rename and resolves on ack', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let ok = false;
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();
        const promise = chat.renameSession('new name');
        await Promise.resolve();
        expect(f.sent.some((s) => s.msg.t === 'session_rename' && s.msg.name === 'new name')).toBe(true);
        f.reply('session_rename', { t: 'ack' });
        await promise;
        ok = true;
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    expect(ok).toBe(true);
    dispose();
  });
});

describe('chat store — send modes, bridge queue, outbox, btw (UNI-159)', () => {
  it('send(mode) wires "now" and "after" through as-is; "auto"/"steer"/"followUp" still compute a pending bubble mode', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: { running: true }, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();

        chat.send('steer this', 'steer');
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'prompt', text: 'steer this', mode: 'steer' });
        expect(chat.state.pending.at(-1)!.mode).toBe('steer');

        chat.send('do it now', 'now');
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'prompt', text: 'do it now', mode: 'now' });
        expect(chat.state.pending.at(-1)!.mode).toBe('prompt');

        chat.send('after this', 'after');
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'prompt', text: 'after this', mode: 'after' });
        // "after" never gets an optimistic pending bubble — it shows in the queue strip instead.
        expect(chat.state.pending.some((p) => p.text === 'after this')).toBe(false);
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
  });

  it('queue{} carries both phone and tui rows, both editable (UNI-202); queueEdit/queueRemove/queueMove/queuePromote send the right wire messages', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();
        f.push({
          t: 'queue',
          items: [
            { id: 'tui-0', text: 'typed in the TUI', mode: 'steer', source: 'tui', editable: true },
            { id: 'q1', text: 'wait for it', mode: 'after', source: 'phone', editable: true },
          ],
        });
        await Promise.resolve();
        expect(chat.state.queue).toHaveLength(2);
        expect(chat.state.queue[0]!.editable).toBe(true);
        expect(chat.state.queue[1]!.editable).toBe(true);

        chat.queueEdit('q1', 'edited text');
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'queue_edit', id: 'q1', text: 'edited text' });

        chat.queueMove('q1', 0);
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'queue_move', id: 'q1', index: 0 });

        chat.queuePromote('q1', 'now');
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'queue_promote', id: 'q1', to: 'now' });

        chat.queueRemove('q1');
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'queue_remove', id: 'q1' });
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
  });

  it("UNI-305: a queue_remove the bridge refuses (a tool is mid-flight) surfaces as a toast, not an aborted run — the queue row stays", async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [{ id: 'tui-1', text: 'steer me', mode: 'steer', source: 'tui', editable: true }] });
        await Promise.resolve();
        expect(chat.state.queue).toHaveLength(1);

        chat.queueRemove('tui-1');
        await Promise.resolve();
        f.reply('queue_remove', { t: 'error', message: "That message was already handed to pi — it can't be taken back while a tool is running." });
        await Promise.resolve();

        expect(chat.state.error?.message).toMatch(/can't be taken back while a tool is running/);
        // The row is still there — nothing was silently dropped or the run aborted.
        expect(chat.state.queue).toHaveLength(1);
        expect(chat.state.queue[0]!.id).toBe('tui-1');
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
  });

  it('restored{texts} lands on state.restored with a fresh timestamp; clearRestored() clears it (UNI-202/211: abort pulls pi\'s own queued messages back)', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();
        expect(chat.state.restored).toBeUndefined();

        f.push({ t: 'restored', texts: ['steer A', 'follow B'] });
        await Promise.resolve();
        expect(chat.state.restored?.texts).toEqual(['steer A', 'follow B']);

        chat.clearRestored();
        await Promise.resolve();
        expect(chat.state.restored).toBeUndefined();
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
  });

  it('queue survives a reconnect: hello.queue repopulates state.queue the same as a live queue{} push (UNI-211)', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();
        f.push({
          t: 'queue',
          items: [{ id: 'tui-0', text: 'steer from TUI', mode: 'steer', source: 'tui', editable: true }],
        });
        await Promise.resolve();
        expect(chat.state.queue).toHaveLength(1);

        // A fresh hello (reconnect, or a second phone joining) carries the
        // same queue — the app shows exactly what the bridge reports, no app-side
        // queue state of its own to go stale.
        f.push({
          t: 'hello',
          session: { id: 's', cwd: '/p' },
          state: {},
          entries: [],
          commands: [],
          models: [],
          dialogs: [],
          queue: [{ id: 'tui-0', text: 'steer from TUI', mode: 'steer', source: 'tui', editable: true }],
        });
        await Promise.resolve();
        expect(chat.state.queue).toHaveLength(1);
        expect(chat.state.queue[0]!.text).toBe('steer from TUI');
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
  });

  it("compact(instructions) sends t:'compact' with the trimmed text (UNI-205: /compact <text> from the phone used to drop the text)", async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();

        chat.compact('focus on the login bug');
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'compact', instructions: 'focus on the login bug' });

        chat.compact();
        await Promise.resolve();
        expect(f.sent.at(-1)!.msg).toMatchObject({ t: 'compact' });
        expect(f.sent.at(-1)!.msg.instructions).toBeUndefined();
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
  });

  it('outbox: a send while not live is persisted and shown as waiting, then flushed in order on hello', async () => {
    localStorage.clear();
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's7');
      // Not live yet (no hello pushed): sends queue into the outbox, not the wire.
      chat.send('first offline message');
      chat.send('second offline message');
      expect(chat.state.outbox).toHaveLength(2);
      expect(f.sent).toHaveLength(0);
      expect(JSON.parse(localStorage.getItem('unipi.chat.outbox.host.s7')!)).toHaveLength(2);

      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();
        await Promise.resolve();
        expect(chat.state.outbox).toHaveLength(0);
        const sentTexts = f.sent.filter((s) => s.msg.t === 'prompt').map((s) => s.msg.text);
        expect(sentTexts).toEqual(['first offline message', 'second offline message']);
        expect(localStorage.getItem('unipi.chat.outbox.host.s7')).toBe('[]');
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
  });

  it('outbox: removeOutboxed drops one message while still waiting for a connection', async () => {
    localStorage.clear();
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's8');
      chat.send('keep me');
      chat.send('drop me');
      const dropId = chat.state.outbox[1]!.id;
      chat.removeOutboxed(dropId);
      expect(chat.state.outbox.map((o) => o.text)).toEqual(['keep me']);
      expect(JSON.parse(localStorage.getItem('unipi.chat.outbox.host.s8')!)).toHaveLength(1);
      return d;
    });
    dispose();
  });

  it('askBtw streams btw_delta into state.btwPages and resolves on btw_end', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();
        const { id, done } = chat.askBtw("what's the codename?");
        await Promise.resolve();
        expect(f.sent.some((s) => s.msg.t === 'btw' && s.msg.question === "what's the codename?")).toBe(true);
        f.push({ t: 'btw_delta', id: 'btw-1', kind: 'text', text: 'Peli', ref: id });
        f.push({ t: 'btw_delta', id: 'btw-1', kind: 'text', text: 'can.', ref: id });
        await Promise.resolve();
        const page = chat.state.btwPages.find((p) => p.id === id);
        expect(page?.answer).toBe('Pelican.');
        expect(page?.done).toBe(false);
        f.push({ t: 'btw_end', id: 'btw-1', answer: 'Pelican.', usage: { input: 5, output: 1, totalTokens: 6 }, ref: id });
        await done;
        const finished = chat.state.btwPages.find((p) => p.id === id);
        expect(finished?.done).toBe(true);
        expect(finished?.usage).toEqual({ input: 5, output: 1, totalTokens: 6 });
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 30));
    dispose();
  });

  it('btwList resolves recent pages from the matching reply', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let result: unknown;
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();
        const promise = chat.btwList();
        await Promise.resolve();
        f.reply('btw_list', { t: 'btw_list', pages: [{ question: 'q', answer: 'a' }] });
        result = await promise;
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    expect(result).toEqual([{ question: 'q', answer: 'a' }]);
    dispose();
  });
});

describe('chat store — fusion preset push (UNI-176)', () => {
  it("a state push with fusion updates the preset; without fusion leaves it alone", async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let seen: unknown;
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();
        const preset = {
          leads: [{ key: 'p/lead', name: 'Lead' }],
          sidekicks: [{ key: 'p/side', name: 'Side' }],
          curated: [],
          default: {},
          effort: {},
          active: { kind: 'fusion', lead: 'p/lead', sidekick: 'p/side' },
        };
        f.push({ t: 'state', running: false, fusion: preset } as never);
        await Promise.resolve();
        seen = [
          chat.state.fusion?.active,
          (chat.state.fusion as { leads?: unknown[] } | undefined)?.leads?.length,
        ];
        // A later state push without fusion must not clear the preset.
        f.push({ t: 'state', running: true } as never);
        await Promise.resolve();
        (seen as unknown[]).push(chat.state.fusion?.active);
        (seen as unknown[]).push(chat.state.run.running);
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    dispose();
    expect(seen).toEqual([
      { kind: 'fusion', lead: 'p/lead', sidekick: 'p/side' },
      1,
      { kind: 'fusion', lead: 'p/lead', sidekick: 'p/side' },
      true,
    ]);
  });
});

describe('btw side questions across reconnects (UNI-219)', () => {
  const hello = { t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] };
  const tick = () => new Promise((r) => setTimeout(r, 0));

  it('reconcileBtwPages: a finished listed page replaces our stuck page (by ref) — no duplicate in the rest', () => {
    const own: BtwPage[] = [{ id: 'r1', question: 'why?', answer: '', done: false }];
    const rest = reconcileBtwPages(own, [{ id: 'btw-1-1', question: 'tui q', answer: 'tui a', done: true, running: false }, { id: 'btw-1-2', ref: 'r1', question: 'why?', answer: 'Because.', done: true, running: false }], true);
    expect(rest.map((p) => p.question)).toEqual(['tui q']);
    expect(own[0]).toMatchObject({ done: true, answer: 'Because.', bridgeId: 'btw-1-2' });
  });

  it('reconcileBtwPages: a running listed page carries progress (answer so far, thinking, tools) and stays running', () => {
    const own: BtwPage[] = [{ id: 'r1', question: 'q', answer: '', done: false }];
    reconcileBtwPages(own, [{ id: 'btw-1-2', ref: 'r1', question: 'q', answer: 'Partial', done: false, running: true, thinking: 'hmm', toolLines: ['… Reading a'] }], true);
    expect(own[0]).toMatchObject({ done: false, answer: 'Partial', thinking: 'hmm', toolLines: ['… Reading a'] });
  });

  it('reconcileBtwPages: older bridges (no ids) match by question; finished older pages are claimed once', () => {
    const own: BtwPage[] = [{ id: 'r1', question: 'same', answer: '', done: false }];
    const rest = reconcileBtwPages(own, [{ question: 'same', answer: 'old answer' }, { question: 'same', answer: 'new answer' }], false);
    // The newest listed match wins for our page only once; the other stays an older page.
    expect(own[0]).toMatchObject({ done: true, answer: 'new answer' });
    expect(rest).toEqual([{ question: 'same', answer: 'old answer' }]);
  });

  it('reconcileBtwPages: complete list without our waiting page → it ends with an error (never "Thinking…" forever)', () => {
    const own: BtwPage[] = [{ id: 'r1', question: 'q', answer: '', done: false }];
    reconcileBtwPages(own, [], true);
    expect(own[0]!.done).toBe(true);
    expect(own[0]!.error).toMatch(/lost/);
    const own2: BtwPage[] = [{ id: 'r2', question: 'q', answer: '', done: false }];
    reconcileBtwPages(own2, [], false);
    expect(own2[0]!.done).toBe(false);
  });

  it('streams thinking and tool lines (a done line replaces its running line by index)', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let chat!: ReturnType<typeof createChat>;
    const dispose = createRoot((d) => {
      chat = createChat('host', 's1');
      return d;
    });
    f.push(hello);
    await tick();
    const { id } = chat.askBtw('q');
    f.push({ t: 'btw_delta', id: 'btw-1-1', kind: 'thinking', text: 'Let me ', ref: id });
    f.push({ t: 'btw_delta', id: 'btw-1-1', kind: 'thinking', text: 'look.', ref: id });
    f.push({ t: 'btw_delta', id: 'btw-1-1', kind: 'tool', text: '… Reading a.ts', index: 0, ref: id });
    f.push({ t: 'btw_delta', id: 'btw-1-1', kind: 'tool', text: '✓ Read a.ts', index: 0, ref: id });
    const page = chat.state.btwPages.find((p) => p.id === id)!;
    expect(page.thinking).toBe('Let me look.');
    expect(page.toolLines).toEqual(['✓ Read a.ts']);
    expect(page.answer).toBe('');
    expect(page.done).toBe(false);
    expect(page.bridgeId).toBe('btw-1-1');
    dispose();
  });

  it('the screenshot bug: the connection drops mid-answer → on hello the store re-lists with watch, the page resumes and finishes in place (one page)', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let chat!: ReturnType<typeof createChat>;
    const dispose = createRoot((d) => {
      chat = createChat('host', 's1');
      return d;
    });
    f.push(hello);
    await tick();
    const { id } = chat.askBtw('why is the sky blue?');
    f.push({ t: 'btw_delta', id: 'btw-7-3', kind: 'thinking', text: 'hmm', ref: id });
    // Reconnected: a new hello arrives (the old socket's stream is gone).
    f.push(hello);
    await tick();
    const list = [...f.sent].reverse().find((s) => s.msg.t === 'btw_list');
    expect(list?.msg.watch).toBe(true);
    f.reply('btw_list', { t: 'btw_list', complete: true, pages: [{ id: 'btw-7-3', ref: id, question: 'why is the sky blue?', answer: 'Ray', done: false, running: true }] });
    await tick();
    expect(chat.state.btwPages).toHaveLength(1);
    expect(chat.state.btwPages[0]).toMatchObject({ answer: 'Ray', done: false });
    // The rest of the stream now reaches the new socket.
    f.push({ t: 'btw_delta', id: 'btw-7-3', kind: 'text', text: 'leigh', ref: id });
    f.push({ t: 'btw_end', id: 'btw-7-3', answer: 'Rayleigh scattering.', ref: id });
    expect(chat.state.btwPages).toHaveLength(1);
    expect(chat.state.btwPages[0]).toMatchObject({ answer: 'Rayleigh scattering.', done: true });
    // Re-opening the sheet: btwList returns only pages that aren't ours.
    const p = chat.btwList();
    await tick();
    f.reply('btw_list', { t: 'btw_list', complete: true, pages: [{ id: 'btw-0-1', question: 'tui', answer: 'x', done: true, running: false }, { id: 'btw-7-3', ref: id, question: 'why is the sky blue?', answer: 'Rayleigh scattering.', done: true, running: false }] });
    expect((await p).map((x) => x.question)).toEqual(['tui']);
    dispose();
  });

  it('the answer finished while disconnected: re-listing turns the stuck page into the answer (no second copy)', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let chat!: ReturnType<typeof createChat>;
    const dispose = createRoot((d) => {
      chat = createChat('host', 's1');
      return d;
    });
    f.push(hello);
    await tick();
    const { id } = chat.askBtw('q');
    const p = chat.btwList();
    await tick();
    // A bridge from before UNI-219: no ids, no flags.
    f.reply('btw_list', { t: 'btw_list', pages: [{ question: 'q', answer: 'A' }] });
    expect(await p).toEqual([]);
    expect(chat.state.btwPages.find((x) => x.id === id)).toMatchObject({ done: true, answer: 'A' });
    dispose();
  });

  it('a btw{} the bridge refuses with error ends that page', async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    let chat!: ReturnType<typeof createChat>;
    const dispose = createRoot((d) => {
      chat = createChat('host', 's1');
      return d;
    });
    f.push(hello);
    await tick();
    const { id } = chat.askBtw('q');
    f.reply('btw', { t: 'error', message: 'btw is not installed on this pi.' });
    expect(chat.state.btwPages.find((x) => x.id === id)).toMatchObject({ done: true, error: 'btw is not installed on this pi.' });
    dispose();
  });
});

describe('customInsertIndex (UNI-251: an old bridge sends the /command echo after its harness message)', () => {
  const at = (id: string, type: string, ts?: string) => ({ type, id, ...(ts ? { timestamp: ts } : {}) }) as never;
  it('a late custom entry goes back before newer entries, by timestamp', () => {
    const list = [at('a', 'message', '2026-10-10T13:00:00Z'), at('u', 'message', '2026-10-10T13:05:24Z'), at('r', 'message', '2026-10-10T13:05:41Z')];
    expect(customInsertIndex(list, at('echo', 'custom', '2026-10-10T13:05:23Z'))).toBe(1);
  });
  it('in-order customs, messages and untimed entries append', () => {
    const list = [at('a', 'message', '2026-10-10T13:00:00Z')];
    expect(customInsertIndex(list, at('c', 'custom', '2026-10-10T13:01:00Z'))).toBe(1);
    expect(customInsertIndex(list, at('m', 'message', '2026-10-10T12:00:00Z'))).toBe(1);
    expect(customInsertIndex(list, at('c', 'custom'))).toBe(1);
    expect(customInsertIndex([at('x', 'message')], at('c', 'custom', '2026-10-10T13:01:00Z'))).toBe(1);
  });
});

describe('tree_go restore (UNI-290)', () => {
  it("treeGo resolves with the ack's restore (text + image refs); an old bridge's bare ack resolves undefined", async () => {
    const f = fakeTransport();
    setChatTransport(f.transport);
    const got: Array<TreeGoRestore | undefined> = [];
    const dispose = createRoot((d) => {
      const chat = createChat('host', 's1');
      void (async () => {
        f.push({ t: 'hello', session: { id: 's', cwd: '/p' }, state: {}, entries: [], commands: [], models: [], dialogs: [], queue: [] });
        await Promise.resolve();
        const a = chat.treeGo('u1');
        await Promise.resolve();
        f.reply('tree_go', { t: 'ack', restore: { text: 'look [image 1]', images: [{ mime: 'image/png', data: 'AAAA', bytes: 3 }, { mime: 'image/jpeg', mediaRef: 'm1-2' }] } });
        got.push(await a);
        const b = chat.treeGo('a1');
        await Promise.resolve();
        f.reply('tree_go', { t: 'ack' });
        got.push(await b);
      })();
      return d;
    });
    await new Promise((r) => setTimeout(r, 20));
    expect(got).toEqual([
      { text: 'look [image 1]', images: [{ mime: 'image/png', data: 'AAAA', bytes: 3 }, { mime: 'image/jpeg', mediaRef: 'm1-2' }] },
      undefined,
    ]);
    dispose();
  });

  it('treeGoRestoreOf ignores malformed restores and image entries with no bytes or ref', () => {
    expect(treeGoRestoreOf(undefined)).toBeUndefined();
    expect(treeGoRestoreOf({ t: 'ack', restore: { text: 5 } })).toBeUndefined();
    expect(treeGoRestoreOf({ t: 'ack', restore: { text: '' } })).toBeUndefined();
    expect(treeGoRestoreOf({ t: 'ack', restore: { text: 'hi', images: [null, { mime: 'image/png' }, { data: 'AA' }] } })).toEqual({ text: 'hi', images: [{ mime: 'image/*', data: 'AA' }] });
  });
});

describe('models push (late providers)', () => {
  it("replaces the model list when the bridge sends 'models' after hello", () => {
    const state = { models: [{ provider: 'openrouter', id: 'a' }] } as unknown as Parameters<typeof applyMessage>[0];
    applyMessage(state, { t: 'models', models: [{ provider: 'openrouter', id: 'a' }, { provider: 'omniroute', id: 'b' }] } as never);
    expect(state.models.map((m: { provider: string }) => m.provider)).toEqual(['openrouter', 'omniroute']);
  });
  it('ignores a malformed push', () => {
    const state = { models: [{ provider: 'x', id: 'y' }] } as unknown as Parameters<typeof applyMessage>[0];
    applyMessage(state, { t: 'models' } as never);
    expect(state.models).toHaveLength(1);
  });
});
