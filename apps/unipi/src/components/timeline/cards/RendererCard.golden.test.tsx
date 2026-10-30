// @vitest-environment jsdom
/**
 * Renderer cards (WP-A8, G-21 acceptance): **every unipi card type from
 * the U7b fixtures** — the nodes the migrated extension render spec owners
 * answer for `_oino/render` — renders through U9's `NodeView` (golden DOM
 * snapshots); an unregistered type keeps the native light row; a renderer
 * that never answers (the 2 s timeout path) also keeps it.
 */
import { cleanup, render, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import type { SessionEntry } from '../../../lib/session/types';
import { resetSessionRenderCaches } from '../../../lib/session/renderCache';
import fixtures from '../../../dev/mock/fixtures/cards.json';
import { RendererCard, nativeCardLabel } from './RendererCard';
import { ModelChangeCard } from './ModelChangeCard';
import { fakeCtx } from '../tools/testutil';

/** The U7b card corpus (fixture key → the customType whose owner answers it). */
const CORPUS: Array<[string, string, 'custom_message' | 'custom']> = [
  ['harnessCard', 'unipi:lh-continue', 'custom_message'],
  ['harnessCard', 'unipi:kanboard-continue', 'custom_message'],
  ['harnessCard', 'unipi:kanboard-next', 'custom_message'],
  ['harnessCard', 'unipi-memory-recall-reminder', 'custom_message'],
  ['harnessCard', 'unipi-memory-retro-reminder', 'custom_message'],
  ['harnessCard', 'unipi-skills-revealed', 'custom_message'],
  ['harnessCard', 'unipi:plan-mode-message', 'custom_message'],
  ['harnessCard', 'compactor-recall', 'custom_message'],
  ['harnessCard', 'unipi-response', 'custom_message'],
  ['harnessWarning', 'unipi-watchdog', 'custom_message'],
  ['subagentCompletion', 'subagent-completion', 'custom_message'],
  ['sidekickCompletion', 'sidekick-completion', 'custom_message'],
  ['backgroundTask', 'background-task-notification', 'custom_message'],
  ['memorySession', 'unipi-memory-session-card', 'custom_message'],
  ['memorySave', 'unipi-memory-save-card', 'custom_message'],
  ['compaction', 'unipi-compaction', 'custom_message'],
  ['kanboardBadge', 'unipi:kanboard-badge', 'custom_message'],
  ['attachments', 'unipi:attachments', 'custom'],
];

const entry = (customType: string, kind: 'custom_message' | 'custom', n: number): SessionEntry =>
  ({
    type: kind,
    id: `c${n}`,
    customType,
    data: { note: 'fixture entry' },
    timestamp: '2026-10-30T09:00:00Z',
  }) as SessionEntry;

const renderersOf = () =>
  Object.fromEntries(CORPUS.map(([fixture, type, kind]) => [`${kind === 'custom' ? 'customEntry' : 'customMessage'}/${type}`, (fixtures as Record<string, unknown>)[fixture]]));

beforeEach(() => resetSessionRenderCaches());
afterEach(cleanup);

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

describe('RendererCard goldens (U7b corpus)', () => {
  it.each(CORPUS)('%s answers for "%s" through NodeView', async (fixture, type, kind) => {
    const ctx = fakeCtx([entry(type, kind, 1)], { renderers: renderersOf() });
    const r = render(() => <RendererCard entry={ctx.session.state.entries[0]!} ctx={ctx} />);
    await waitFor(() => expect(r.getByTestId('tl-card-render')).toBeTruthy(), { timeout: 2000 });
    expect(r.queryByTestId('tl-card-render')).toBeTruthy();
    expect(normalize(r.container.firstElementChild!)).toMatchSnapshot();
  });

  it('every card type of the corpus is covered (guards the fixture file)', () => {
    const fixtureKeys = new Set(Object.keys(fixtures).filter((k) => !k.startsWith('_')));
    const covered = new Set(CORPUS.map(([f]) => f));
    expect([...fixtureKeys].filter((k) => !covered.has(k) && k !== 'toolBashAdvanced')).toEqual([]);
  });
});

describe("the frontend's own row", () => {
  it('an unregistered type keeps the native light row (source · title)', () => {
    const ctx = fakeCtx([entry('acme:report', 'custom_message', 2)], {});
    const r = render(() => <RendererCard entry={ctx.session.state.entries[0]!} ctx={ctx} />);
    expect(r.container.querySelector('.tl-card')).toBeTruthy();
    expect(r.container.textContent).toContain('acme:report');
  });

  it('a renderer that never answers (2 s timeout path) renders nothing over the native row', async () => {
    const ctx = fakeCtx([entry('acme:stuck', 'custom_message', 3)], { renderers: { 'customMessage/acme:stuck': { never: true } } });
    const r = render(() => <RendererCard entry={ctx.session.state.entries[0]!} ctx={ctx} />);
    // still native while the request hangs — and nothing else renders
    await new Promise((res) => setTimeout(res, 20));
    expect(r.container.querySelector('.tl-card')).toBeTruthy();
    expect(r.queryByTestId('tl-card-render')).toBeNull();
  });

  it('harness meta beats entry data for the native label', () => {
    const e = { type: 'custom_message', customType: 'unipi:lh-continue', data: {}, message: { details: { unipiHarness: { source: 'Long-horizon', title: 'Continuation' } } } } as unknown as SessionEntry;
    expect(nativeCardLabel(e)).toEqual({ source: 'Long-horizon', title: 'Continuation' });
  });
});

describe('native change cards', () => {
  it('model_change renders the light "Model → X" row', () => {
    const r = render(() => <ModelChangeCard entry={{ type: 'model_change', id: 'm1', modelId: 'claude-sonnet-4-5' } as SessionEntry} />);
    expect(r.getByTestId('tl-model-change').textContent).toContain('Model → claude-sonnet-4-5');
  });
  it('thinking_level_change renders "Thinking → high"', () => {
    const r = render(() => <ModelChangeCard entry={{ type: 'thinking_level_change', id: 'm2', level: 'high' } as SessionEntry} />);
    expect(r.getByTestId('tl-model-change').textContent).toContain('Thinking → high');
  });
});
