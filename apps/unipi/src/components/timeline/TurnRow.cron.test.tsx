// @vitest-environment jsdom
/** [ui-chat-bits] A cron-run chat marks its first user message "Scheduled run" (only that one). */
import { cleanup, render } from '@solidjs/testing-library';
import { afterEach, expect, it } from 'vitest';
import { buildTurns } from '../../lib/session/turns';
import type { SessionEntry } from '../../lib/session/types';
import type { TimelineCtx } from './context';
import { TurnRow } from './TurnRow';

afterEach(() => cleanup());

const msg = (id: string, parentId: string | null, role: 'user' | 'assistant', text: string) =>
  ({ type: 'message', id, parentId, timestamp: new Date(0).toISOString(), message: { role, content: [{ type: 'text', text }], timestamp: 1 } }) as unknown as SessionEntry;
const entries = [msg('u0', null, 'user', 'Summarize'), msg('a0', 'u0', 'assistant', 'Done'), msg('u1', 'a0', 'user', 'Thanks'), msg('a1', 'u1', 'assistant', 'OK')];
const turns = buildTurns({ entries, leafId: 'a1', entriesRev: 0, liveTools: {}, run: { running: false } } as never);

const ctxOf = (cron: boolean) =>
  ({
    session: { state: { byId: {}, entries: [] }, key: 'k', turns: () => turns },
    hostId: 'h',
    sessionId: () => 's',
    cwd: () => '/',
    refs: { hostId: 'h', cwd: () => '/' },
    commands: () => new Set<string>(),
    columnWidth: () => 720,
    cronRun: () => cron,
  }) as unknown as TimelineCtx;

it('a cron chat: the first turn says "Scheduled run", later turns say nothing', () => {
  const userTurns = turns.filter((t) => t.kind === 'turn');
  const first = render(() => <TurnRow turn={userTurns[0]!} ctx={ctxOf(true)} />);
  expect(first.getByTestId('tl-cron').textContent).toContain('Scheduled run');
  cleanup();
  const second = render(() => <TurnRow turn={userTurns[1]!} ctx={ctxOf(true)} />);
  expect(second.queryByTestId('tl-cron')).toBeNull();
});

it('a user chat: no marker', () => {
  const r = render(() => <TurnRow turn={turns.find((t) => t.kind === 'turn')!} ctx={ctxOf(false)} />);
  expect(r.queryByTestId('tl-cron')).toBeNull();
});
