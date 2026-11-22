// @vitest-environment jsdom
/**
 * The chat header's ⋯ menu (owner 2026-11-06): the "Chat filter" entry opens
 * the audience filter; the three checkboxes are the audiences' rows in the
 * timeline, and they are an app-wide setting.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { SettingsProvider, useSettingsApi, type SettingsApi } from '../../settings/context';
import { createMemoryAdapter } from '../../settings/adapter';
import { setDesktopOverride } from '../../lib/layout';
import { ConversationHeader } from './ConversationHeader';

const ctx = { hostId: 'this-pc', sessionId: 's1', title: 'A chat' };

let api: SettingsApi | undefined;
function Probe() {
  api = useSettingsApi();
  return null;
}

const header = () =>
  render(() => (
    <SettingsProvider adapter={createMemoryAdapter(undefined)}>
      <Probe />
      <ConversationHeader title="A chat" ctx={ctx} onNewChat={() => {}} desktop />
    </SettingsProvider>
  ));

afterEach(() => {
  cleanup();
  api = undefined;
  setDesktopOverride(undefined);
});

describe('the chat filter menu (owner 2026-11-06)', () => {
  it('offers the three audiences and toggles them', async () => {
    setDesktopOverride(true);
    header();
    fireEvent.click(screen.getByTestId('conv-more'));
    await waitFor(() => expect(screen.getByTestId('conv-chat-filter')).toBeTruthy());
    fireEvent.click(screen.getByTestId('conv-chat-filter'));

    // the three checkboxes, with the defaults: user + shared on, agent-only off
    await waitFor(() => expect(screen.getByTestId('conv-filter-user')).toBeTruthy());
    expect(screen.getByTestId('conv-filter-user').getAttribute('aria-checked')).toBe('true');
    expect(screen.getByTestId('conv-filter-both').getAttribute('aria-checked')).toBe('true');
    expect(screen.getByTestId('conv-filter-agent').getAttribute('aria-checked')).toBe('false');

    // the agent-only rows can be shown
    fireEvent.click(screen.getByTestId('conv-filter-agent'));
    await waitFor(() => expect(api!.settings.chat.showAudienceAgent).toBe(true));
    await waitFor(() => expect(screen.getByTestId('conv-filter-agent').getAttribute('aria-checked')).toBe('true'));
  });
});
