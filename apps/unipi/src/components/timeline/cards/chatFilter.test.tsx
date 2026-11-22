// @vitest-environment jsdom
/**
 * The chat filter (owner 2026-11-06): the ⋯ menu's "Chat filter" hides the
 * event rows of an audience the user turned off. Agent-only rows are hidden by
 * default; normal user/assistant messages are never filtered.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { CHAT_EVENT_CORPUS } from '../../../dev/mock/fixtures/chatEvents';
import { setDesktopOverride } from '../../../lib/layout';
import { resetSessionRenderCaches } from '../../../lib/session/renderCache';
import { SettingsProvider } from '../../../settings/context';
import { createMemoryAdapter } from '../../../settings/adapter';
import { fakeCtx } from '../tools/testutil';
import { CardRow, audienceShown } from './CardRow';
import type { SessionEntry } from '../../../lib/session/types';

const entry = (id: string): SessionEntry => CHAT_EVENT_CORPUS.find(([, e]) => e.id === id)![1] as unknown as SessionEntry;

/** The corpus rows of each audience: agent-only (display:false), user-only, both. */
const AGENT_ONLY = 'ev02';
const USER_ONLY = 'ev05';
const BOTH = 'ev28';

const renderRow = (e: SessionEntry, chat?: Record<string, boolean>) => {
  const adapter = createMemoryAdapter(chat ? { version: 1, chat } : undefined);
  const ctx = fakeCtx([e]);
  return render(() => (
    <SettingsProvider adapter={adapter}>
      <CardRow entry={e} ctx={ctx} />
    </SettingsProvider>
  ));
};

afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  resetSessionRenderCaches();
});

describe('the chat filter (owner 2026-11-06)', () => {
  it('hides the agent-only rows by default and keeps the other two audiences', async () => {
    const agent = renderRow(entry(AGENT_ONLY));
    await waitFor(() => expect(agent.queryByTestId('tl-event')).toBeNull());
    cleanup();

    const user = renderRow(entry(USER_ONLY));
    await waitFor(() => expect(user.getByTestId('tl-event')).toBeTruthy());
    cleanup();

    const both = renderRow(entry(BOTH));
    await waitFor(() => expect(both.getByTestId('tl-event')).toBeTruthy());
  });

  it('shows the agent-only rows when the filter says so', async () => {
    const r = renderRow(entry(AGENT_ONLY), { showAudienceUser: true, showAudienceBoth: true, showAudienceAgent: true });
    await waitFor(() => expect(r.getByTestId('tl-event')).toBeTruthy());
  });

  it('hides a row of a turned-off audience', async () => {
    const r = renderRow(entry(USER_ONLY), { showAudienceUser: false, showAudienceBoth: true, showAudienceAgent: false });
    await waitFor(() => expect(r.queryByTestId('tl-event')).toBeNull());
  });

  it('keeps every audience when there are no settings at all', () => {
    expect(audienceShown('user', undefined)).toBe(true);
    expect(audienceShown('agent', undefined)).toBe(true);
    expect(audienceShown('both', undefined)).toBe(true);
  });
});
