// @vitest-environment jsdom
/**
 * [chat-load] Chat stability harness (owner 2026-11-04: "entering a chat is extremely
 * unstable: the screen flashes, rows overlap, giant gaps, scrolling teleports me").
 *
 * Drives the real Timeline + session store over an owner-shaped paged chat with a flow
 * layout that behaves like a browser (flowLayout.testutil): rows first lay out at one
 * height and settle to another a frame later (markdown, cards), the host pages by entries
 * like `page_start`. The reader scrolls up through ALL history by wheel, then by scrollbar
 * drag, flicks, and expands / collapses folds. Every frame:
 *
 *   - no overlap: mounted row rects never intersect, and rows are contiguous (no gaps);
 *   - anchor: in frames without reader input (chunk loads, rows settling, folds) the first
 *     visible turn moves by at most 2 px;
 *   - bounded: chunk loads ≤ the chunks the history needs; row mounts and settles bounded;
 *     no load while bottom-locked or while the reader is still flicking.
 */
import { cleanup, fireEvent, render } from '@solidjs/testing-library';
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createMockHost, type MockScenario } from '../../lib/host/mock';
import type { SessionSnapshot } from '../../lib/host/client';
import type { SessionEntry } from '../../lib/session/types';
import { createSession, type SessionHandle } from '../../lib/session/store';
import { sessionRefChat } from '../../lib/timeline/refChat';
import { clearScrollMemory } from '../../lib/timeline/scrollMemory';
import { clearTimelineSnapshots } from '../../lib/timeline/snapshotCache';
import { clearSessionHeights } from '../../lib/timeline/rowModel';
import { dropSessionExpansion } from '../../lib/timeline/expansion';
import { commandNames, type TimelineCtx } from './context';
import { Timeline } from './Timeline';
import { installFlowLayout, type FlowLayout } from './flowLayout.testutil';

vi.mock('../../lib/host/factory', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../lib/host/factory')>()),
  hostClient: () => {
    throw new Error('no ambient host');
  },
}));

/* ── an owner-shaped branch: 60 turns, two giant subagent turns (one with a steer), compactions ── */
function ownerBranch(): SessionEntry[] {
  const out: Array<Record<string, unknown>> = [];
  let t = Date.parse('2026-11-03T03:00:00Z');
  const push = (e: Record<string, unknown>) => out.push({ id: `e${out.length}`, timestamp: new Date((t += 1000)).toISOString(), ...e });
  const user = (text: string) => push({ type: 'message', message: { role: 'user', content: [{ type: 'text', text }], timestamp: t } });
  const answer = (text: string) => push({ type: 'message', message: { role: 'assistant', content: [{ type: 'text', text }], stopReason: 'stop', timestamp: t } });
  const tool = (id: string, name: string) => {
    push({ type: 'message', message: { role: 'assistant', content: [{ type: 'toolCall', id, name, arguments: { command: 'ls', path: 'x' } }], stopReason: 'toolUse', timestamp: t } });
    push({ type: 'message', message: { role: 'toolResult', toolCallId: id, toolName: name, content: [{ type: 'text', text: 'ok' }], isError: false, timestamp: t } });
  };
  for (let turn = 0; turn < 60; turn++) {
    user(`Question ${turn}`);
    if (turn === 20 || turn === 45) {
      // a giant turn: subagent runs + hundreds of child steps, a steer in the middle
      const agent = `ag${turn}`;
      push({ type: 'message', message: { role: 'assistant', content: [{ type: 'toolCall', id: `run-${agent}`, name: 'run_subagent', arguments: { title: `wl-${agent}`, profile: 'subagent_general', task: 'x' } }], stopReason: 'toolUse', timestamp: t } });
      for (let i = 0; i < 260; i++) push({ type: 'custom', customType: 'subagent-step', data: { kind: i % 3 ? 'tool' : 'text', name: 'read', arg: `f${i}.rs`, output: 'ok', text: `step ${i}`, agentId: agent, id: agent, title: `wl-${agent}` } });
      push({ type: 'message', message: { role: 'toolResult', toolCallId: `run-${agent}`, toolName: 'run_subagent', content: [{ type: 'text', text: 'done' }], isError: false, timestamp: t } });
      user('steer: also check the docs');
      for (let i = 0; i < 6; i++) tool(`c${turn}-${i}`, i % 2 ? 'read' : 'bash');
      answer(`Long answer ${turn}`);
    } else if (turn % 9 === 4) {
      push({ type: 'compaction', summary: 'compacted', firstKeptEntryId: 'e0', tokensBefore: 1000 });
      answer(`Answer ${turn}`);
    } else {
      for (let i = 0; i < turn % 4; i++) tool(`c${turn}-${i}`, 'bash');
      answer(`Answer ${turn}`);
    }
  }
  out.forEach((e, i) => (e.parentId = i ? out[i - 1]!.id : null));
  return out as unknown as SessionEntry[];
}

const isUser = (e: SessionEntry) => e.type === 'message' && e.message?.role === 'user';
const pageStart = (all: SessionEntry[], end: number, limit: number) => {
  let start = Math.max(0, end - limit);
  if (start > 0) for (let a = start; a < end; a++) if (isUser(all[a]!)) return a;
  return start;
};

let L: FlowLayout;
const grown = new Set<string>();
/** Base height first, the settled height one frame later (markdown / cards rendering). */
const finalH = (id: string) => (id === 'pre:e0' ? 60 : 120 + ((id.length * 37 + Number(id.replace(/\D/g, '') || 0) * 13) % 380));
let disposers: Array<() => void> = [];

beforeEach(() => {
  localStorage.clear();
  clearScrollMemory();
  clearTimelineSnapshots();
  clearSessionHeights();
  grown.clear();
  L = installFlowLayout({ viewport: 700, rowHeight: (id) => (grown.has(id) ? finalH(id) : 90) });
});
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  cleanup();
  L.restore();
  dropSessionExpansion('s1');
});

async function open() {
  const all = ownerBranch();
  const tailFrom = pageStart(all, all.length, 80);
  const asked: Array<{ before: string; limit: number }> = [];
  const snapshot = {
    session_id: 's1',
    reset: true,
    seq: 1,
    state: { sessionId: 's1', isStreaming: false, model: { provider: 'p', id: 'm', name: 'm' } },
    entries: all.slice(tailFrom),
    leaf_id: all.at(-1)!.id,
    page: { total: all.length, has_more: true, cursor: all[tailFrom]!.id },
  } as unknown as SessionSnapshot;
  const scenario: MockScenario = {
    name: 'stability',
    hosts: [
      {
        id: 'h1',
        name: 'pc',
        sessions: {
          s1: {
            snapshot,
            events: [],
            cmd: {
              get_entries: (cmd: Record<string, unknown>) => {
                if (typeof cmd.before !== 'string') return { entries: [], leafId: all.at(-1)!.id };
                asked.push({ before: cmd.before, limit: Number(cmd.limit) });
                const end = all.findIndex((e) => e.id === cmd.before);
                const from = pageStart(all, end, Number(cmd.limit ?? 80));
                return { entries: all.slice(from, end), total: all.length, hasMore: from > 0, cursor: all[from]?.id };
              },
            },
          },
        },
      },
    ],
  };
  const world = createMockHost(scenario);
  const { s, dispose } = createRoot((d) => ({ s: createSession('h1', 's1', { client: world.client('h1') }), dispose: d }));
  disposers.push(() => {
    s.close();
    dispose();
  });
  for (let i = 0; i < 20 && s.state.conn !== 'live'; i++) await new Promise((r) => setTimeout(r, 0));
  return { s, all, asked };
}

function ctx(s: SessionHandle): TimelineCtx {
  const cwd = () => '/proj';
  return { session: s, hostId: 'h1', sessionId: s.sessionId, cwd, refs: sessionRefChat(s, cwd), commands: () => commandNames(s.state.commands), columnWidth: () => 720 };
}

/** Per-frame checks; `quiet` = no reader input this frame (the anchor must hold). */
function checker(tl: HTMLElement) {
  const report = { frames: 0, overlaps: 0, gaps: 0, maxQuietJump: 0, worst: '', lockedOffBottom: 0 };
  const locked = () => !tl.querySelector('[data-testid="tl-jump"]');
  let last: { id: string; top: number } | undefined;
  const visibleFirst = () => L.rows(tl).find((r) => r.bottom > 0.5);
  return {
    report,
    before() {
      const f = visibleFirst();
      last = f ? { id: f.id, top: f.top } : undefined;
    },
    after(quiet: boolean, what: string) {
      report.frames++;
      const rows = L.rows(tl);
      for (let i = 1; i < rows.length; i++) {
        if (rows[i]!.top < rows[i - 1]!.bottom - 0.5) report.overlaps++;
        if (rows[i]!.top > rows[i - 1]!.bottom + 0.5) report.gaps++;
      }
      // Bottom-locked: the anchor is the bottom (rows above may settle), it must hold exactly.
      if (quiet && locked()) {
        if (tl.scrollHeight - tl.clientHeight - tl.scrollTop > 0.5) report.lockedOffBottom++;
      } else if (quiet && last) {
        const now = rows.find((r) => r.id === last!.id);
        if (now) {
          const jump = Math.abs(now.top - last.top);
          if (jump > report.maxQuietJump) {
            report.maxQuietJump = jump;
            report.worst = `${what}: ${last.id} ${last.top.toFixed(1)} → ${now.top.toFixed(1)}`;
          }
        }
      }
    },
    /** Rows mounted at the last paint finish rendering in the next frame (before its paint). */
    grow() {
      for (const r of L.rows(tl)) grown.add(r.id);
    },
  };
}

describe('chat stability (owner chat shape): scroll through all history', () => {
  it('wheel up to the very start, then drag, flick and folds: no overlap, no gap, anchor ≤ 2 px, bounded loads and renders', async () => {
    const { s, all, asked } = await open();
    const r = render(() => <Timeline ctx={ctx(s)} />);
    const tl = r.getByTestId('tl');
    let mounts = 0;
    const mo = new MutationObserver((ms) => {
      for (const m of ms) for (const n of m.addedNodes) if ((n as HTMLElement).classList?.contains('tl-vrow')) mounts++;
    });
    mo.observe(tl, { childList: true, subtree: true });
    const c = checker(tl);
    /** A paint boundary: every microtask (store commits, settles) has run, like before a browser paint. */
    const paint = () => new Promise((res) => setTimeout(res, 0));
    const quietFrames = async (n: number, what: string) => {
      for (let i = 0; i < n; i++) {
        await paint();
        c.before();
        c.grow();
        await L.frame();
        await paint();
        c.after(true, what);
      }
    };

    // ── open: whole turns, pinned, nothing split ──
    await quietFrames(12, 'open');
    expect(tl.dataset.ready).toBe('');
    expect(s.turns().some((t) => t.turnId.startsWith('pre:'))).toBe(false);
    expect(tl.scrollTop).toBe(tl.scrollHeight - tl.clientHeight);
    const openLoads = asked.length;
    // locked + idle: nothing loads by itself
    await new Promise((res) => setTimeout(res, 250));
    await quietFrames(6, 'locked idle');
    expect(asked.length).toBe(openLoads);

    // ── wheel up through ALL history: steps of 350 px; at the top, wait for the scroll to go idle ──
    let guard = 0;
    while ((s.state.history?.hasMore || tl.scrollTop > 0) && guard++ < 400) {
      c.grow();
      await L.wheelTo(tl, Math.max(0, tl.scrollTop - 350));
      c.after(false, 'wheel');
      if (tl.scrollTop < 1.5 * tl.clientHeight) {
        const loadsBefore = asked.length;
        await new Promise((res) => setTimeout(res, 200)); // the scroll goes idle → one chunk
        await quietFrames(6, 'chunk load');
        expect(asked.length - loadsBefore).toBeLessThanOrEqual(10); // ONE chunk (≤ 10 host pages)
      }
    }
    expect(s.state.history?.hasMore).toBe(false);
    expect(s.state.entries.length).toBe(all.length);
    expect(s.turns()[0]!.turnId).toBe('e0');
    expect(tl.scrollTop).toBe(0);
    // every chunk was whole turns: no turn id ever started at a steer
    expect(s.turns().some((t) => t.user && (t.user.message as unknown as { content: Array<{ text: string }> }).content[0]!.text.startsWith('steer'))).toBe(false);

    // ── scrollbar drag down to the middle and back up (no wheel: pointer on the scroller) ──
    fireEvent.pointerDown(tl);
    for (let y = 0; y < tl.scrollHeight / 2; y += 900) {
      tl.scrollTop = y; // the browser moves it; we only see the scroll event
      c.grow();
      await L.frame();
      c.after(false, 'drag');
    }
    await quietFrames(4, 'drag settle');

    // ── flick: fast scroll events every frame; nothing loads mid-flick ──
    const flickLoads = asked.length;
    for (let i = 0; i < 20; i++) {
      tl.scrollTop = tl.scrollTop - 250;
      c.grow();
      await L.frame();
      c.after(false, 'flick');
      expect(asked.length).toBe(flickLoads);
    }
    await quietFrames(4, 'flick settle');

    // ── folds: expand and collapse a mounted fold; its header holds still ──
    const head = [...tl.querySelectorAll<HTMLElement>('.tl-vrow .proc-head')].find((h) => {
      const top = h.getBoundingClientRect().top;
      return top > 50 && top < 600;
    });
    expect(head).toBeTruthy();
    const row = head!.closest<HTMLElement>('.tl-vrow')!;
    const id = row.dataset.turn!;
    for (const open of [true, false, true]) {
      const at = head!.getBoundingClientRect().top;
      fireEvent.click(head!);
      const base = finalH(id);
      L.rowHeight = (x) => (x === id ? (open ? base + 640 : base) : grown.has(x) ? finalH(x) : 90);
      c.before();
      c.grow();
      await L.frames();
      c.after(true, open ? 'expand' : 'collapse');
      expect(Math.abs(head!.getBoundingClientRect().top - at)).toBeLessThanOrEqual(2);
    }

    mo.disconnect();
    const turns = s.turns().length;
    const { report } = c;
    // eslint-disable-next-line no-console
    console.info('[stability]', JSON.stringify({ ...report, loads: asked.length, mounts, turns, paging: (window as unknown as { __TL_PAGING__: () => unknown }).__TL_PAGING__() }));
    expect(report.overlaps).toBe(0);
    expect(report.gaps).toBe(0);
    expect(report.maxQuietJump).toBeLessThanOrEqual(2);
    expect(report.lockedOffBottom).toBe(0);
    // host pages: ~3000 entries / 200 per page, plus one re-fetch per chunk boundary
    expect(asked.length).toBeLessThanOrEqual(Math.ceil(all.length / 200) * 2 + 4);
    // row mounts: each turn mounts a bounded number of times over the whole run (up, drag down, flick up)
    expect(mounts).toBeLessThanOrEqual(turns * 4);
  }, 30_000);
});
