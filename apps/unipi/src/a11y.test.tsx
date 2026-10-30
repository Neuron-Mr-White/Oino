/**
 * Accessibility sweep (WP-A28, Z-116): the app's shared rules, asserted over
 * the *real* shell rendered from the capture scenarios (`src/dev/mock`), not
 * over stand-alone components:
 *
 *   - every control has an accessible name;
 *   - every icon-only control has a tooltip (the shared 300 ms one, Z-116);
 *   - tooltip copy is plain English — no emoji, no bare glyph (QA#10);
 *   - exits (Z-91) are 160 ms, and none at all under reduced motion;
 *   - the empty chat state greets by local time and never shows recents (Z-87).
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createRoot, createSignal } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Root } from './main';
import { createMockHost } from './dev/mock/host';
import type { MockScenario } from './dev/mock/host';
import { SCENARIOS } from './dev/mock/scenarios';
import { setHostClientFactory } from './lib/host/factory';
import { resetLayout, setDesktopOverride } from './lib/layout';
import { setPlatformOverride } from './lib/platform';
import { resetPanes } from './lib/panes';
import { resetHubs } from './lib/session/hub';
import { resetSessions } from './lib/session/store';
import { EXIT_MS, createPresence, exitDurationMs, prefersReducedMotion } from './lib/motion';
import { fakeCtx } from './components/timeline/tools/testutil';
import { ToolRow } from './components/timeline/tools/ToolRow';
import { ThinkingRow } from './components/timeline/tools/ThinkingRow';
import { TurnActions } from './components/timeline/TurnActions';
import type { Turn } from './lib/session/turns';
import type { TimelineCtx } from './components/timeline/context';
import { TIP_DELAY_MS, TIP_EXIT_MS, Tooltip, TooltipHost, accessibleName, currentTip, isIconOnly, tooltipTextFor } from './components/Tooltip';
import { GREETING_TEXT, greetingFor, greetingSlot } from './lib/greeting';
import { loreLine } from './lib/lore';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

function width(w: number) {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: w });
}

/** Renders the whole app on a scenario (the capture harness's world). */
function boot(scenarioName: string, hash: string, opts: { w?: number; phone?: boolean } = {}) {
  const scenario = SCENARIOS[scenarioName] as MockScenario;
  width(opts.w ?? 1440);
  setPlatformOverride(opts.phone ? 'tauri-mobile' : 'tauri-desktop');
  setDesktopOverride(!opts.phone);
  setHostClientFactory(createMockHost(scenario).client);
  location.hash = hash;
  return render(() => <Root />);
}

/** The scenario's first host + card, for a conversation route. */
function route(scenarioName: string): string {
  const scenario = SCENARIOS[scenarioName] as MockScenario;
  const host = scenario.hosts[0]!;
  const session = host.cards?.[0]?.session_id ?? Object.keys(host.sessions ?? {})[0] ?? 's001';
  return `#/s/${host.id}/${session}`;
}

const CONTROLS = 'button, a[href], [role="button"], [role="tab"], [role="menuitem"], [role="switch"]';

/** The controls a screen shows, with the reason each one fails (if it does). */
function problems(root: ParentNode): string[] {
  const out: string[] = [];
  for (const el of root.querySelectorAll(CONTROLS)) {
    const name = accessibleName(el);
    const html = el.outerHTML.replace(/\s+/g, ' ').slice(0, 180);
    if (!name) out.push(`no accessible name: ${html}`);
    else if (isIconOnly(el) && !tooltipTextFor(el)) out.push(`icon-only, no tooltip: ${html}`);
  }
  return out;
}

async function expectChrome(scenarioName: string, hash: string, opts: { w?: number; phone?: boolean; ready?: () => Promise<unknown> } = {}) {
  const view = boot(scenarioName, hash, opts);
  if (opts.ready) await opts.ready();
  // Wait for the real chrome: a first paint has only a couple of controls.
  await waitFor(() => expect(document.querySelectorAll(CONTROLS).length).toBeGreaterThan(5), { timeout: 5000 });
  await new Promise((r) => setTimeout(r, 120));
  const found = problems(view.container);
  expect(found, `${scenarioName} @ ${hash}\n${found.join('\n')}`).toEqual([]);
  return view;
}

beforeEach(() => {
  localStorage.clear();
});
afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  setDesktopOverride(undefined);
  setPlatformOverride(undefined);
  setHostClientFactory(undefined);
  resetHubs();
  resetSessions();
  resetPanes();
  resetLayout();
  location.hash = '';
  width(1024);
});

describe('shared tooltip (Z-116)', () => {
  it('takes its text from data-tip, else the accessible name of an icon-only control', () => {
    const doc = document.implementation.createHTMLDocument('t');
    doc.body.innerHTML = `
      <button id="a" aria-label="New chat"><svg></svg></button>
      <button id="b" data-tip="Start a chat in this folder" aria-label="New chat"><svg></svg></button>
      <button id="c">Close</button>
      <button id="d" aria-label="Copy"><svg></svg>Copy</button>`;
    const tip = (id: string) => tooltipTextFor(doc.getElementById(id)!);
    expect(tip('a')).toBe('New chat');
    expect(tip('b')).toBe('Start a chat in this folder');
    expect(tip('c')).toBeUndefined(); // text says it already
    expect(tip('d')).toBeUndefined(); // has a visible word
    expect(isIconOnly(doc.getElementById('a')!)).toBe(true);
    expect(isIconOnly(doc.getElementById('c')!)).toBe(false);
  });

  it('waits 300 ms per trigger, restarts for the next one, and shows at once on focus', async () => {
    vi.useFakeTimers();
    try {
      const view = render(() => (
        <>
          <button aria-label="One" title="One"><svg /></button>
          <button aria-label="Two" title="Two"><svg /></button>
          <TooltipHost />
        </>
      ));
      const [one, two] = Array.from(view.container.querySelectorAll('button')) as HTMLButtonElement[];
      fireEvent.pointerOver(one!, { pointerType: 'mouse' });
      expect(currentTip()).toBeUndefined();
      vi.advanceTimersByTime(TIP_DELAY_MS - 20);
      expect(currentTip()).toBeUndefined();
      vi.advanceTimersByTime(30);
      expect(currentTip()?.text).toBe('One');
      expect(screen.getByRole('tooltip').textContent).toBe('One');
      expect(one!.hasAttribute('title')).toBe(false); // one tooltip, not two
      // The trigger carries aria-describedby while the tip is up.
      expect(one!.getAttribute('aria-describedby')).toBe('unipi-tooltip');

      // Moving to another trigger starts a fresh 300 ms.
      fireEvent.pointerOut(one!, { relatedTarget: two });
      fireEvent.pointerOver(two!, { pointerType: 'mouse' });
      expect(currentTip()).toBeUndefined();
      vi.advanceTimersByTime(TIP_DELAY_MS);
      expect(currentTip()?.text).toBe('Two');
      expect(one!.hasAttribute('aria-describedby')).toBe(false);
      expect(one!.getAttribute('title')).toBe('One'); // the native one is back

      // Escape dismisses it (through its short exit fade).
      fireEvent.keyDown(document, { key: 'Escape' });
      expect(currentTip()?.leaving).toBe(true);
      vi.advanceTimersByTime(TIP_EXIT_MS);
      expect(currentTip()).toBeUndefined();

      // Keyboard focus needs no delay.
      fireEvent.focusIn(two!);
      expect(currentTip()?.text).toBe('Two');
      fireEvent.focusOut(two!);
      expect(currentTip()?.leaving).toBe(true);
      vi.advanceTimersByTime(TIP_EXIT_MS);
      expect(currentTip()).toBeUndefined();
    } finally {
      vi.useRealTimers();
    }
  });

  it('works on a disabled control through the <Tooltip> wrapper', async () => {
    vi.useFakeTimers();
    try {
      render(() => (
        <>
          <Tooltip label="Nothing to undo yet">
            <button disabled aria-label="Undo">
              <svg />
            </button>
          </Tooltip>
          <TooltipHost />
        </>
      ));
      const wrap = document.querySelector('.tip') as HTMLElement;
      const button = wrap.querySelector('button')!;
      // The wrapper says something better than the control's name ("Undo"),
      // and it is what the host shows for the trigger inside it.
      expect(tooltipTextFor(wrap)).toBe('Nothing to undo yet');
      expect(tooltipTextFor(button)).toBe('Nothing to undo yet');
      fireEvent.pointerOver(wrap, { pointerType: 'mouse' });
      vi.advanceTimersByTime(TIP_DELAY_MS);
      expect(currentTip()?.text).toBe('Nothing to undo yet');
    } finally {
      vi.useRealTimers();
    }
  });

  it('is plain English: no emoji, no bare glyph', () => {
    const el = document.createElement('button');
    el.setAttribute('aria-label', '✕');
    expect(tooltipTextFor(el)).toBeUndefined();
    el.setAttribute('aria-label', 'Close panel');
    expect(tooltipTextFor(el)).toBeUndefined(); // no icon: not icon-only, no explicit tip
    el.setAttribute('data-tip', 'Close panel');
    expect(tooltipTextFor(el)).toBe('Close panel');
    el.setAttribute('data-tip', '🧹 Clear');
    expect(tooltipTextFor(el)).toBeUndefined();
  });
});

describe('exit motion (Z-91)', () => {
  it('keeps a closing element mounted for the exit, then drops it', async () => {
    vi.useFakeTimers();
    try {
      const [open, setOpen] = createSignal(true);
      const seen: string[] = [];
      const dispose = createRoot((d) => {
        const p = createPresence(open);
        const read = () => {
          seen.push(`${p.mounted() ? 'mounted' : 'gone'}${p.leaving() ? '+leaving' : ''}`);
        };
        read();
        // Collect each state the presence passes through.
        return { p, read, dispose: d };
      });
      expect(seen).toEqual(['mounted']);
      setOpen(false);
      expect(dispose.p.mounted()).toBe(true);
      expect(dispose.p.leaving()).toBe(true);
      expect(EXIT_MS).toBeLessThanOrEqual(200);
      vi.advanceTimersByTime(EXIT_MS);
      expect(dispose.p.mounted()).toBe(false);
      expect(dispose.p.leaving()).toBe(false);
      dispose.dispose();
    } finally {
      vi.useRealTimers();
    }
  });

  it('has no exit at all under prefers-reduced-motion', () => {
    const original = window.matchMedia;
    window.matchMedia = ((q: string) => ({ matches: q.includes('prefers-reduced-motion'), media: q, addEventListener() {}, removeEventListener() {}, addListener() {}, removeListener() {}, onchange: null, dispatchEvent: () => false })) as unknown as typeof window.matchMedia;
    try {
      expect(prefersReducedMotion()).toBe(true);
      expect(exitDurationMs()).toBe(0);
      const [open, setOpen] = createSignal(true);
      let p!: ReturnType<typeof createPresence>;
      const dispose = createRoot((d) => {
        p = createPresence(open);
        return d;
      });
      setOpen(false);
      expect(p.mounted()).toBe(false);
      expect(p.leaving()).toBe(false);
      dispose();
    } finally {
      window.matchMedia = original;
    }
  });
});

describe('new-chat empty state (Z-87)', () => {
  it('greets by the device clock in six slots', () => {
    const at = (h: number) => greetingFor(new Date(2026, 10, 1, h, 30));
    expect(at(6)).toBe(GREETING_TEXT.early);
    expect(at(10)).toBe(GREETING_TEXT.morning);
    expect(at(12)).toBe(GREETING_TEXT.noon);
    expect(at(16)).toBe(GREETING_TEXT.afternoon);
    expect(at(19)).toBe(GREETING_TEXT.evening);
    expect(at(2)).toBe(GREETING_TEXT.late);
    expect(new Set([0, 5, 9, 12, 14, 18, 23].map(greetingSlot))).toEqual(new Set(['late', 'early', 'morning', 'noon', 'afternoon', 'evening']));
  });

  it('picks a lore line deterministically, never the same slot twice in a row', () => {
    expect(loreLine(0)).toBe(loreLine(0));
    expect(loreLine(1)).not.toBe(loreLine(0));
    expect(loreLine(6)).toBe(loreLine(0));
    expect(loreLine(0).length).toBeGreaterThan(20);
  });

  it('shows the crab, the greeting, one hint and a lore line — no recents', async () => {
    const scenario = SCENARIOS['empty-chat'] as MockScenario;
    const host = scenario.hosts[0]!;
    const view = boot('empty-chat', `#/new/${host.id}`);
    await waitFor(() => expect(view.getByTestId('empty-chat')).toBeTruthy());
    const box = view.getByTestId('empty-chat');
    expect(box.querySelector('svg')).toBeTruthy(); // the Unicrab rig
    expect(view.getByTestId('empty-chat-greeting').textContent).toBe(greetingFor(new Date()));
    expect(box.querySelectorAll('p').length).toBe(2); // hint + lore, nothing else
    expect(view.container.querySelector('[data-testid="empty-chat"] button')).toBeNull(); // no recent-session list
    expect(problems(view.container), problems(view.container).join('\n')).toEqual([]);
  });
});

describe('the app shell has no unnamed or untooltipped control', () => {
  /** The shell is up (a first paint has only a couple of controls). */
  const chrome = () => waitFor(() => expect(document.querySelectorAll(CONTROLS).length).toBeGreaterThan(5), { timeout: 5000 });

  it('conversation, sidebar and status (shell-desktop)', async () => {
    await expectChrome('shell-desktop', route('shell-desktop'), { ready: chrome });
  }, 15_000);

  it('settings (settings-nav)', async () => {
    await expectChrome('settings-nav', '#/settings', { ready: chrome });
  }, 15_000);

  it('board (board-dashboard)', async () => {
    await expectChrome('board-dashboard', '#/board', { ready: chrome });
  }, 15_000);

  it('phone chat tab (sidebar-states)', async () => {
    await expectChrome('sidebar-states', '#/chat', { w: 412, phone: true, ready: chrome });
  }, 15_000);

  it('phone files layer (files-tree)', async () => {
    await expectChrome('files-tree', '#/files', { w: 412, phone: true, ready: chrome });
  }, 15_000);

  it('transcript rows: tool, thinking and turn actions', async () => {
    // The virtualizer mounts no rows in jsdom (no layout), so the transcript's
    // own controls render here over A8's fake ctx — the same rows the timeline
    // shows once geometry exists.
    const ctx = fakeCtx([]) as TimelineCtx;
    (ctx.session.state as { run: unknown }).run = { running: false, phase: 'idle' };
    const tool = { id: 't1', name: 'bash', args: { command: 'npm test' }, state: 'ok', output: 'ok' };
    const turn = { kind: 'turn', turnId: 'u1', entries: [], segments: [], stopped: false } as unknown as Turn;
    const view = render(() => (
      <>
        <ToolRow tool={tool as never} ctx={ctx} itemKey="k1" />
        <ThinkingRow item={{ key: 'a1-0', text: 'Weighing the options.', live: false }} ctx={ctx} />
        <TurnActions turn={turn} ctx={ctx} last />
      </>
    ));
    const found = problems(view.container);
    expect(found, found.join('\n')).toEqual([]);
    // Guard against a vacuous pass: the rows really rendered their buttons.
    expect(view.container.querySelectorAll(CONTROLS).length).toBeGreaterThanOrEqual(3);
  });
});
