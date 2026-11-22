// @vitest-environment jsdom
/**
 * [chat-events] Event row goldens (the RendererCard golden pattern): every
 * kind of the chat event corpus renders through `CardRow` as its own
 * `EventCard` — icon, "Source · title", meta, tone — collapsed, then
 * expanded to its body (DOM snapshots of both). Kinds whose extension draws
 * the body take the `_oino/render` node; hidden kinds render nothing; the
 * phone opens the body in a sheet instead of growing the row.
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { CHAT_EVENT_CORPUS } from '../../../dev/mock/fixtures/chatEvents';
import fixtures from '../../../dev/mock/fixtures/cards.json';
import { resetSessionRenderCaches } from '../../../lib/session/renderCache';
import { setDesktopOverride } from '../../../lib/layout';
import { dropSessionExpansion } from '../../../lib/timeline/expansion';
import { fakeCtx } from '../tools/testutil';
import { CardRow } from './CardRow';
import { describeEvent } from '../../../lib/timeline/eventKinds';

const setWidth = (w: number) => {
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: w });
  setDesktopOverride(w >= 900);
};

function normalize(el: Element): Element {
  const clone = el.cloneNode(true) as Element;
  const ids = new Map<string, string>();
  const norm = (v: string) => v.replace(/\b(cl-\d+|solid-\S+-\d+|[a-z]+-\d+-\d+)\b/g, (m) => ids.get(m) ?? (ids.set(m, `id${ids.size}`), ids.get(m)!));
  for (const node of [clone, ...clone.querySelectorAll('*')]) {
    for (const attr of ['id', 'for', 'aria-labelledby', 'aria-controls', 'aria-describedby', 'aria-activedescendant']) {
      const v = node.getAttribute(attr);
      if (v) node.setAttribute(attr, norm(v));
    }
  }
  return clone;
}

const visible = CHAT_EVENT_CORPUS.filter(([k]) => k !== null).map(([kind, e]) => [`${kind} (${e.id})`, kind!, e] as const);
const hidden = CHAT_EVENT_CORPUS.filter(([k]) => k === null).map(([, e]) => [e.customType!, e] as const);

beforeEach(() => {
  resetSessionRenderCaches();
  dropSessionExpansion('s1');
  setWidth(1440);
});
afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
});

describe('EventCard goldens (chat event corpus)', () => {
  it.each(visible)('%s: collapsed one-liner, then the expanded body', async (_name, kind, entry) => {
    const ctx = fakeCtx([entry]);
    const r = render(() => <CardRow entry={entry} ctx={ctx} />);
    const row = r.getByTestId('tl-event');
    expect(row.getAttribute('data-event')).toBe(kind);
    const head = r.getByTestId('tl-event-head');
    if (!describeEvent(entry)!.body && !describeEvent(entry)!.renderer) {
      // a plain marker (Mode · Graph): no chevron, nothing to expand
      expect(head.getAttribute('aria-disabled')).toBe('true');
      expect(row.querySelector('.tl-event-chev')).toBeNull();
      expect(normalize(row)).toMatchSnapshot('marker');
      return;
    }
    expect(head.getAttribute('aria-expanded')).toBe('false');
    expect(r.queryByTestId('tl-event-body')).toBeNull();
    expect(normalize(row)).toMatchSnapshot('collapsed');
    fireEvent.click(head);
    expect(head.getAttribute('aria-expanded')).toBe('true');
    // the extension renderer answers null here → the app's own body
    await waitFor(() => expect(row.querySelector('.tl-event-body')).toBeTruthy());
    await new Promise((res) => setTimeout(res, 0));
    expect(row.querySelector('.tl-event-body')!.textContent!.trim()).not.toBe('');
    expect(normalize(row)).toMatchSnapshot('expanded');
    fireEvent.click(head);
    expect(head.getAttribute('aria-expanded')).toBe('false');
  });

  it.each(hidden)('%s renders nothing', (_t, entry) => {
    const ctx = fakeCtx([entry]);
    const r = render(() => <CardRow entry={entry} ctx={ctx} />);
    expect(r.queryByTestId('tl-event')).toBeNull();
    expect(r.container.textContent).toBe('');
  });

  it('a kind whose extension draws the body shows the extension node once expanded (asked with expanded:true)', async () => {
    const entry = CHAT_EVENT_CORPUS.find(([k]) => k === 'dream-report')![1];
    const asked: unknown[] = [];
    const ctx = fakeCtx([entry], { renderers: { 'customEntry/unipi-dream-report-card': fixtures.memorySave } });
    const cmd = ctx.session.cmd.bind(ctx.session);
    (ctx.session as { cmd: unknown }).cmd = (c: { type: string }) => {
      asked.push(c);
      return cmd(c as never);
    };
    const r = render(() => <CardRow entry={entry} ctx={ctx} />);
    expect(asked).toEqual([]); // nothing asked while collapsed
    fireEvent.click(r.getByTestId('tl-event-head'));
    await waitFor(() => expect(r.getByTestId('tl-event-render')).toBeTruthy());
    expect((asked[0] as { expanded?: boolean }).expanded).toBe(true);
    expect(r.getByTestId('tl-event-render').textContent).toContain('Saved 2 memories');
  });

  it('the open state survives a remount (virtual scroll) through the expansion store', () => {
    const entry = CHAT_EVENT_CORPUS.find(([k]) => k === 'memory-saved')![1];
    const ctx = fakeCtx([entry]);
    const a = render(() => <CardRow entry={entry} ctx={ctx} />);
    fireEvent.click(a.getByTestId('tl-event-head'));
    a.unmount();
    const b = render(() => <CardRow entry={entry} ctx={ctx} />);
    expect(b.getByTestId('tl-event-head').getAttribute('aria-expanded')).toBe('true');
    expect(b.getByTestId('tl-event-body')).toBeTruthy();
  });

  it('JSON details render in a code block, never as prose', async () => {
    const entry = CHAT_EVENT_CORPUS.find(([k]) => k === 'custom')![1];
    const r = render(() => <CardRow entry={entry} ctx={fakeCtx([entry])} />);
    fireEvent.click(r.getByTestId('tl-event-head'));
    await waitFor(() => expect(r.getByTestId('tl-event-body')).toBeTruthy());
    const body = r.getByTestId('tl-event-body');
    expect(body.tagName).toBe('PRE');
    expect(body.getAttribute('data-format')).toBe('json');
    expect(body.textContent).toContain('"rows": 3');
  });

  it('phone: the row stays one line and the body opens in a sheet', async () => {
    setWidth(390);
    const entry = CHAT_EVENT_CORPUS.find(([k]) => k === 'memory-recall')![1];
    const r = render(() => <CardRow entry={entry} ctx={fakeCtx([entry])} />);
    fireEvent.click(r.getByTestId('tl-event-head'));
    expect(r.getByTestId('tl-event').querySelector('.tl-event-bodywrap')).toBeNull();
    await waitFor(() => expect(document.querySelector('[data-testid="event-sheet"]')).toBeTruthy());
    expect(document.querySelector('[data-testid="event-sheet"]')!.textContent).toContain('oino_handoff_20261102_finalize_graph');
  });
});
