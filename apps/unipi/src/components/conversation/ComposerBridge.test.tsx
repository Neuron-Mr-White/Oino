// @vitest-environment jsdom
/**
 * The A7 composer handoff (Z-28/Z-30): quote chips and an edit-restored
 * draft (text + images) show in the composer, send as a blockquote + image
 * payload on the MockHost, and clear afterwards.
 */
import { cleanup, fireEvent, render, waitFor } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createMockHost, type MockScenario, type MockWorld } from '../../lib/host/mock';
import type { HostClient, SessionSnapshot } from '../../lib/host/client';
import { createSession, type SessionHandle } from '../../lib/session/store';
import { addComposerQuote, clearComposer, restoreComposer } from '../../lib/timeline/actions';
import { ComposerBridge } from './ComposerBridge';
import roundtrip from '../../lib/session/fixtures/roundtrip.json';

const rt = roundtrip as unknown as { fresh: SessionSnapshot };

vi.mock('../../lib/host/factory', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../lib/host/factory')>()),
  hostClient: () => clientOf(),
}));
let clientOf: () => HostClient = () => {
  throw new Error('clientOf not set');
};

beforeEach(() => {
  localStorage.clear();
  clearComposer('h1/s1');
});

let disposers: Array<() => void> = [];
afterEach(() => {
  disposers.forEach((d) => d());
  disposers = [];
  cleanup();
});

async function open(): Promise<{ s: SessionHandle; w: MockWorld }> {
  const scenario: MockScenario = { name: 't', hosts: [{ id: 'h1', name: 'coffee', sessions: { s1: { snapshot: structuredClone(rt.fresh), events: [] } } }] };
  const w = createMockHost(scenario);
  clientOf = () => w.client('h1');
  const { s, dispose } = createRoot((d) => ({
    s: createSession('h1', 's1', {
      client: w.client('h1'),
      schedule: (f) => f(),
      setTimer: (ms, cb) => setTimeout(cb, ms) as unknown as number,
      clearTimer: (h) => clearTimeout(h as number),
      now: () => Date.now(),
    }),
    dispose: d,
  }));
  disposers.push(() => {
    s.close();
    dispose();
  });
  await waitFor(() => expect(s.state.conn).toBe('live'));
  return { s, w };
}

describe('ComposerBridge', () => {
  it('shows quote chips, sends them as a blockquote and clears the draft (Z-30)', async () => {
    const { s, w } = await open();
    const r = render(() => (
      <HashRouter>
        <Route path="*" component={() => <ComposerBridge session={s} />} />
      </HashRouter>
    ));
    const input = r.getByTestId('tl-composer').querySelector('input') as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'my follow-up' } });
    addComposerQuote(s.key, 'quoted selection');
    await waitFor(() => expect(r.getByTestId('tl-quote-chip').textContent).toContain('quoted selection'));

    fireEvent.click(r.getByLabelText('Send'));
    await waitFor(() => expect(w.log.some((e) => e.op === 'prompt')).toBe(true));
    expect(w.log.find((e) => e.op === 'prompt')!.args).toMatchObject({ type: 'prompt', message: '> quoted selection\n\nmy follow-up' });
    await waitFor(() => expect(r.queryByTestId('tl-quote-chip')).toBeNull());
    expect(input.value).toBe('');
  });

  it('shows an edit-restored draft with its images and sends the image payload (Z-28)', async () => {
    const { s, w } = await open();
    const r = render(() => (
      <HashRouter>
        <Route path="*" component={() => <ComposerBridge session={s} />} />
      </HashRouter>
    ));
    restoreComposer(s.key, { text: 'Question one', images: [{ mimeType: 'image/png', data: 'AAAA' }] });
    const input = await waitFor(() => r.getByTestId('tl-composer').querySelector('input') as HTMLInputElement);
    expect(input.value).toBe('Question one');
    expect(r.getByTestId('tl-draft-image').textContent).toContain('image 1');

    fireEvent.click(r.getByLabelText('Send'));
    await waitFor(() => expect(w.log.some((e) => e.op === 'prompt')).toBe(true));
    expect(w.log.find((e) => e.op === 'prompt')!.args).toMatchObject({
      type: 'prompt',
      message: 'Question one',
      images: [{ type: 'image', data: 'AAAA', mimeType: 'image/png' }],
    });
    await waitFor(() => expect(r.queryByTestId('tl-draft-image')).toBeNull());
  });

  it('/cron opens the reserved Cron page instead of sending a text dump (G-17, WP-A23)', async () => {
    const { s, w } = await open();
    const r = render(() => (
      <HashRouter>
        <Route path="*" component={() => <ComposerBridge session={s} />} />
      </HashRouter>
    ));
    const input = r.getByTestId('tl-composer').querySelector('input') as HTMLInputElement;
    fireEvent.input(input, { target: { value: '/cron add' } });
    fireEvent.click(r.getByLabelText('Send'));
    await waitFor(() => expect(location.hash).toBe('#/cron?form=new'));
    // Nothing was sent to the session — the page *is* the command output.
    expect(w.log.some((e) => e.op === 'prompt')).toBe(false);
    expect(input.value).toBe('');

    fireEvent.input(input, { target: { value: '/cron' } });
    fireEvent.click(r.getByLabelText('Send'));
    await waitFor(() => expect(location.hash).toBe('#/cron'));
    expect(w.log.some((e) => e.op === 'prompt')).toBe(false);
  });
});
