/** UNI-200: the Unicrab renderer and the floating crab (drag/snap, tap, long-press, bubble). */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Route, MemoryRouter } from '@solidjs/router';
import { Unicrab, runs } from './Unicrab';
import { sprite } from '../lib/unicrab';
import { needsYou } from '../lib/needsYou';
import { BUBBLE_MS, LONG_PRESS_MS } from '../lib/mascot';

vi.mock('../hosts/context', () => ({
  useHosts: () => ({ hosts: () => [{ id: 'h1', name: 'Workstation', ticket: 'x', paired_at: 0, last_path: 'lan' }] }),
}));
const opened: string[] = [];
vi.mock('../lib/notifyInit', () => ({ openSessionRoute: (hostId: string, sessionId: string) => opened.push(`#/s/${hostId}/${sessionId}`) }));

import { FloatingMascot } from './FloatingMascot';

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  localStorage.clear();
  needsYou().clearAll();
  opened.length = 0;
});

describe('<Unicrab>', () => {
  it('draws every part as crisp rects in its own animated group', () => {
    const { container } = render(() => <Unicrab state="working" size={60} />);
    const svg = container.querySelector('svg.unicrab')!;
    expect(svg.getAttribute('data-state')).toBe('working');
    expect(svg.getAttribute('shape-rendering')).toBe('crispEdges');
    expect(svg.getAttribute('width')).toBe('60');
    for (const t of ['all', 'torso', 'claw-left', 'claw-right', 'legs-left', 'legs-right'])
      expect(svg.querySelector(`[data-track="${t}"]`), t).toBeTruthy();
    for (const m of ['open', 'left', 'right', 'blink', 'closed', 'happy']) expect(svg.querySelector(`[data-eyes="${m}"]`), m).toBeTruthy();
    // rects are whole cells
    for (const r of svg.querySelectorAll('rect')) {
      expect(Number.isInteger(Number(r.getAttribute('x')))).toBe(true);
      expect(r.getAttribute('height')).toBe('1');
    }
    expect(screen.getByRole('img')).toHaveAccessibleName('Unicrab: Working');
    // the keyframes are injected once
    expect(document.querySelectorAll('style[data-unicrab]')).toHaveLength(1);
    render(() => <Unicrab state="idle" />);
    expect(document.querySelectorAll('style[data-unicrab]')).toHaveLength(1);
  });

  it('sleep shows the Z\'s; other states don\'t', () => {
    const a = render(() => <Unicrab state="sleep" />);
    expect(a.container.querySelector('[data-track="zzz"]')).toBeTruthy();
    cleanup();
    const b = render(() => <Unicrab state="idle" />);
    expect(b.container.querySelector('[data-track="zzz"]')).toBeNull();
  });

  it('idle blinks on a random timer (data-blink on, then off)', () => {
    vi.useFakeTimers();
    vi.spyOn(Math, 'random').mockReturnValue(0);
    const { container } = render(() => <Unicrab state="idle" />);
    const svg = container.querySelector('svg')!;
    expect(svg.hasAttribute('data-blink')).toBe(false);
    vi.advanceTimersByTime(2201);
    expect(svg.hasAttribute('data-blink')).toBe(true);
    vi.advanceTimersByTime(150);
    expect(svg.hasAttribute('data-blink')).toBe(false);
    vi.restoreAllMocks();
  });

  it('reduced motion: never blinks', () => {
    vi.useFakeTimers();
    window.matchMedia = ((q: string) => ({ matches: q.includes('reduce'), media: q, addEventListener() {}, removeEventListener() {} })) as unknown as typeof window.matchMedia;
    const { container } = render(() => <Unicrab state="idle" />);
    vi.advanceTimersByTime(20_000);
    expect(container.querySelector('svg')!.hasAttribute('data-blink')).toBe(false);
    delete (window as { matchMedia?: unknown }).matchMedia;
  });

  it('runs() merges same-colour neighbours on a row', () => {
    expect(runs([[0, 0, 1], [1, 0, 1], [2, 0, 2], [4, 0, 2], [0, 1, 1]])).toEqual([
      { x: 0, y: 0, w: 2, c: 1 },
      { x: 2, y: 0, w: 1, c: 2 },
      { x: 4, y: 0, w: 1, c: 2 },
      { x: 0, y: 1, w: 1, c: 1 },
    ]);
    const total = Object.values(sprite.parts).flat().length;
    expect(runs(Object.values(sprite.parts).flat()).length).toBeLessThan(total);
  });
});

function mount(onOpenDrawer = vi.fn(), onHide = vi.fn()) {
  render(() => (
    <MemoryRouter>
      <Route path="*" component={() => <FloatingMascot onOpenDrawer={onOpenDrawer} onHide={onHide} />} />
    </MemoryRouter>
  ));
  return { onOpenDrawer, onHide, float: () => screen.getByTestId('mascot-float'), button: () => screen.getByRole('button', { name: /Needs you/ }) };
}

const down = (el: Element, x: number, y: number) => fireEvent.pointerDown(el, { button: 0, clientX: x, clientY: y, pointerId: 1 });
const move = (el: Element, x: number, y: number) => fireEvent.pointerMove(el, { clientX: x, clientY: y, pointerId: 1 });
const up = (el: Element, x: number, y: number) => fireEvent.pointerUp(el, { clientX: x, clientY: y, pointerId: 1 });

describe('<FloatingMascot>', () => {
  beforeEach(() => {
    Object.defineProperty(window, 'innerWidth', { configurable: true, value: 412 });
    Object.defineProperty(window, 'innerHeight', { configurable: true, value: 915 });
  });

  it('starts on the right edge; a tap opens the drawer', async () => {
    const m = mount();
    expect(m.float().dataset.side).toBe('right');
    expect(m.float().style.left).toBe(`${412 - 56 - 6}px`);
    down(m.button(), 380, 30);
    up(m.button(), 381, 31);
    expect(m.onOpenDrawer).toHaveBeenCalledTimes(1);
  });

  it('dragging moves it, release snaps to the nearer side and remembers it', () => {
    const m = mount();
    const b = m.button();
    down(b, 380, 30);
    move(b, 200, 300);
    expect(m.float().hasAttribute('data-dragging')).toBe(true);
    move(b, 40, 400);
    up(b, 40, 400);
    expect(m.onOpenDrawer).not.toHaveBeenCalled();
    expect(m.float().dataset.side).toBe('left');
    expect(m.float().style.left).toBe('6px');
    expect(JSON.parse(localStorage.getItem('unipi.mascot.pos.v1')!).side).toBe('left');
  });

  it('long-press opens the quick actions (no drawer); Hide calls onHide', () => {
    vi.useFakeTimers();
    const m = mount();
    down(m.button(), 380, 30);
    vi.advanceTimersByTime(LONG_PRESS_MS + 10);
    up(m.button(), 380, 30);
    expect(m.onOpenDrawer).not.toHaveBeenCalled();
    expect(screen.getByRole('menu', { name: 'Unicrab quick actions' })).toBeInTheDocument();
    fireEvent.click(screen.getByRole('menuitem', { name: /Hide the crab/ }));
    expect(m.onHide).toHaveBeenCalled();
  });

  it('a new finished item pops the summary bubble; tapping it opens that chat; it fades away on its own', () => {
    vi.useFakeTimers();
    mount();
    expect(screen.queryByTestId('mascot-bubble')).toBeNull();
    needsYou().upsert({ id: 'h1:7:agent_end:5', hostId: 'h1', hostName: 'Workstation', pid: 7, session: 'auth', title: 'auth rewrite', kind: 'agent_end', group: 'finished', what: 'auth: token refresh done, 42 tests pass.', since: 5 });
    const bubble = screen.getByTestId('mascot-bubble');
    expect(bubble).toHaveTextContent('auth rewrite');
    expect(bubble).toHaveTextContent('auth: token refresh done, 42 tests pass.');
    fireEvent.click(bubble);
    expect(opened).toHaveLength(1);
    expect(opened[0]).toBe('#/s/h1/auth');
    expect(needsYou().isRead('h1:7:agent_end:5')).toBe(true);
    expect(screen.queryByTestId('mascot-bubble')).toBeNull();

    needsYou().upsert({ id: 'h1:8:ask_user:6', hostId: 'h1', hostName: 'Workstation', pid: 8, session: 's', title: 'docs', kind: 'ask_user', group: 'waiting', what: 'Which plan?', since: 6 });
    expect(screen.getByTestId('mascot-bubble')).toHaveTextContent('Which plan?');
    vi.advanceTimersByTime(BUBBLE_MS + 10);
    expect(screen.queryByTestId('mascot-bubble')).toBeNull();
  });

  it('badge shows the unread count', () => {
    mount();
    needsYou().upsert({ id: 'x', hostId: 'h1', hostName: 'W', pid: 1, session: 's', title: 't', kind: 'ask_user', group: 'waiting', since: 1 });
    expect(screen.getByRole('button', { name: 'Needs you (1)' })).toHaveTextContent('1');
  });
});
