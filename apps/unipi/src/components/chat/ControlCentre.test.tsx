// @vitest-environment jsdom
/**
 * Session control centre pieces (UNI-160, UNI-188): `footerStripText`,
 * `runningWorkCount` and the subheader's inline `FooterTray`.
 */
import { render, screen, cleanup } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createStore } from 'solid-js/store';
import { FooterStrip, FooterTray, footerStripText, runningWorkCount } from './ControlCentre';
import type { Chat, WorkItem } from '../../lib/chat/store';

afterEach(cleanup);

function fakeChat(overrides: Record<string, unknown> = {}): Chat {
  const [state] = createStore({
    conn: 'live',
    run: { running: false, model: { provider: 'anthropic', id: 'claude', name: 'Claude' }, thinking: 'medium', context: { tokens: 1200, window: 10000, percent: 12 } },
    entries: [],
    truncated: false,
    tools: {},
    commands: [],
    models: [],
    dialogs: [],
    queue: [],
    pending: [],
    hasOlder: false,
    loadingOlder: false,
    seams: [],
    work: [] as WorkItem[],
    ...overrides,
  });
  return { state } as unknown as Chat;
}

describe('runningWorkCount (UNI-188)', () => {
  it('counts only items whose dot is "running"', () => {
    const chat = fakeChat({
      work: [
        { id: '1', kind: 'bg', title: 'a', dot: 'running', startedAt: 0, canStop: true, canRerun: false, canBackground: false },
        { id: '2', kind: 'bg', title: 'b', dot: 'done', startedAt: 0, canStop: false, canRerun: true, canBackground: false },
        { id: '3', kind: 'subagent', title: 'c', dot: 'running', startedAt: 0, canStop: true, canRerun: false, canBackground: false },
      ],
    });
    expect(runningWorkCount(chat)).toBe(2);
  });
  it('is zero with no work', () => {
    expect(runningWorkCount(fakeChat())).toBe(0);
  });
});

describe('footerStripText (UNI-160)', () => {
  it('reads mode, model, context % and cost into one line', () => {
    const chat = fakeChat({ stats: { cost: 1.2, tokensIn: 0, tokensOut: 0, cacheHit: 0, tps: 0 } });
    expect(footerStripText(chat)).toBe('Single · Claude · ctx 12% · $1.20');
  });
});

describe('FooterStrip (UNI-188)', () => {
  it('shows the "N running" badge only while background work is running', () => {
    render(() => <FooterStrip chat={fakeChat()} open={false} onToggle={() => {}} />);
    expect(screen.queryByTestId('footer-strip-badge')).toBeNull();

    cleanup();
    const chat = fakeChat({
      work: [
        { id: '1', kind: 'bg', title: 'a', dot: 'running', startedAt: 0, canStop: true, canRerun: false, canBackground: false },
        { id: '2', kind: 'subagent', title: 'b', dot: 'running', startedAt: 0, canStop: true, canRerun: false, canBackground: false },
      ],
    });
    render(() => <FooterStrip chat={chat} open={false} onToggle={() => {}} />);
    expect(screen.getByTestId('footer-strip-badge').textContent).toContain('2 running');
  });

  it('tapping the strip calls onToggle (the tray expand/collapse)', () => {
    const onToggle = vi.fn();
    render(() => <FooterStrip chat={fakeChat()} open={false} onToggle={onToggle} />);
    screen.getByLabelText('Session details').click();
    expect(onToggle).toHaveBeenCalledTimes(1);
  });

  it('reflects the open state via aria-expanded', () => {
    render(() => <FooterStrip chat={fakeChat()} open onToggle={() => {}} />);
    expect(screen.getByLabelText('Session details').getAttribute('aria-expanded')).toBe('true');
  });
});

describe('FooterTray (UNI-188, trimmed in UNI-201)', () => {
  it('is hidden (aria-hidden, no data-open) when closed, shown when open', () => {
    const chat = fakeChat();
    render(() => <FooterTray chat={chat} open={false} onOpenModel={() => {}} onOpenSection={() => {}} />);
    const tray = screen.getByTestId('chat-footer-tray');
    expect(tray.getAttribute('aria-hidden')).toBe('true');
    expect(tray.hasAttribute('data-open')).toBe(false);

    cleanup();
    render(() => <FooterTray chat={chat} open onOpenModel={() => {}} onOpenSection={() => {}} />);
    const open = screen.getByTestId('chat-footer-tray');
    expect(open.getAttribute('aria-hidden')).toBe('false');
    expect(open.hasAttribute('data-open')).toBe(true);
  });

  it('shows model+thinking on one row, context, cost, a single Running summary and the waiting-on label — never the item list', () => {
    const chat = fakeChat({
      run: {
        running: false,
        model: { provider: 'anthropic', id: 'claude', name: 'Claude' },
        thinking: 'high',
        context: { tokens: 3400, window: 20000, percent: 17 },
        waiting: '2 bg tasks will resume agent',
      },
      work: [
        { id: '1', kind: 'bg', title: 'Build the thing', dot: 'running', startedAt: 0, canStop: true, canRerun: false, canBackground: false },
        { id: '2', kind: 'bg', title: 'Done thing', dot: 'done', startedAt: 0, canStop: false, canRerun: true, canBackground: false },
      ],
      stats: { cost: 0.42, tokensIn: 0, tokensOut: 0, cacheHit: 0, tps: 0 },
    });
    render(() => <FooterTray chat={chat} open onOpenModel={() => {}} onOpenSection={() => {}} />);
    expect(screen.getByText('Claude · high')).toBeTruthy();
    expect(screen.getByText('17% · 3400/20000')).toBeTruthy();
    expect(screen.getByText('$0.42')).toBeTruthy();
    expect(screen.getByText('1 running · 1 done')).toBeTruthy();
    expect(screen.queryByText('Build the thing')).toBeNull();
    expect(screen.getByText('Working… · 2 bg tasks will resume agent')).toBeTruthy();
  });

  it('tapping the Model row calls onOpenModel; tapping the Running row opens the Running section', () => {
    const chat = fakeChat({
      work: [{ id: '1', kind: 'bg', title: 'Build the thing', dot: 'running', startedAt: 0, canStop: true, canRerun: false, canBackground: false }],
    });
    const onOpenModel = vi.fn();
    const onOpenSection = vi.fn();
    render(() => <FooterTray chat={chat} open onOpenModel={onOpenModel} onOpenSection={onOpenSection} />);
    screen.getByText('Claude · medium').click();
    expect(onOpenModel).toHaveBeenCalledTimes(1);
    screen.getByText('1 running').click();
    expect(onOpenSection).toHaveBeenCalledWith('running');
  });
});
