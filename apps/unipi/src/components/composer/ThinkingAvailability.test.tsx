// @vitest-environment jsdom
import { cleanup, render, screen, waitFor } from '@solidjs/testing-library';
import { createStore } from 'solid-js/store';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ThinkingChip } from '../model/ThinkingChip';
import { rememberModels } from '../draft/hostCache';
import { resetThinkingLevels } from '../../lib/model/thinking';
import { initialState } from '../../lib/session/reducer';
import type { SessionHandle } from '../../lib/session/store';

const models = [
  { provider: 'p', id: 'first', reasoning: true },
  { provider: 'p', id: 'second', reasoning: true, thinkingLevelMap: { high: null } },
  { provider: 'p', id: 'plain', reasoning: false },
];
function session(reply: () => Promise<unknown>) {
  const [state, set] = createStore({ ...initialState('h', 'draft'), conn: 'live' as const, model: { provider: 'p', id: 'first' }, thinkingLevel: 'low' });
  const cmd = vi.fn((c: { type: string }) => c.type === 'get_available_models' ? Promise.resolve({ models }) : reply());
  return { handle: { key: 'h/draft', hostId: 'h', state, cmd } as unknown as SessionHandle, set };
}
afterEach(() => { cleanup(); resetThinkingLevels(); localStorage.clear(); });
describe('thinking availability', () => {
  it('shows catalog levels on a draft before the capability reply', () => {
    rememberModels('h', models);
    const { handle } = session(() => new Promise(() => {}));
    render(() => <ThinkingChip session={handle} />);
    expect(screen.getByTestId('thinking-chip')).toBeTruthy();
  });
  it.each(['failure', 'timeout'])('keeps catalog levels on lookup %s', async (kind) => {
    const { handle } = session(() => kind === 'failure' ? Promise.reject(new Error('offline')) : new Promise(() => {}));
    render(() => <ThinkingChip session={handle} />);
    await waitFor(() => expect(screen.getByTestId('thinking-chip')).toBeTruthy());
  });
  it('changes to the new model catalog immediately and hides for a non-reasoning model', async () => {
    rememberModels('h', models);
    const { handle, set } = session(() => new Promise(() => {}));
    render(() => <ThinkingChip session={handle} />);
    expect(screen.getByTestId('thinking-chip')).toBeTruthy();
    set('model', { provider: 'p', id: 'second' });
    expect(screen.getByTestId('thinking-chip')).toBeTruthy();
    set('model', { provider: 'p', id: 'plain' });
    expect(screen.queryByTestId('thinking-chip')).toBeNull();
  });
  it('remains visible during a run', () => {
    rememberModels('h', models);
    const { handle, set } = session(() => new Promise(() => {}));
    set('run', 'running', true);
    render(() => <ThinkingChip session={handle} />);
    expect(screen.getByTestId('thinking-chip')).toBeTruthy();
  });
});
