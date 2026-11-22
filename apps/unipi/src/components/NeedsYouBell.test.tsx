// @vitest-environment jsdom
/** The desktop shell bell ([ui-shell] #5): red unread dot + count in the accessible label. */
import { cleanup, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { ShellNeedsYouBell } from './NeedsYouBell';
import { needsYou, needsYouId, resetNeedsYou, type NeedsYouItem } from '../lib/needsYou';

const item = (id: string, since: number): NeedsYouItem => ({
  id: needsYouId('h1', 10, id, since),
  hostId: 'h1',
  hostName: 'My PC',
  pid: 10,
  session: 'default',
  title: 'my session',
  kind: 'ask_user',
  group: 'waiting',
  since,
});

beforeEach(() => {
  localStorage.clear();
  resetNeedsYou();
});
afterEach(() => {
  cleanup();
  resetNeedsYou();
});

describe('shell bell', () => {
  it('shows a red dot while unread, the count in its label, and clears once read', async () => {
    render(() => <ShellNeedsYouBell onOpen={() => {}} />);
    const bell = screen.getByTestId('shell-needs-you');
    expect(bell.getAttribute('aria-label')).toBe('Notifications');
    expect(screen.queryByTestId('shell-needs-you-dot')).toBeNull();
    needsYou().setHostItems('h1', [item('a', 1000), item('b', 2000)]);
    await waitFor(() => expect(bell.getAttribute('aria-label')).toBe('Notifications, 2 unread'));
    expect(screen.getByTestId('shell-needs-you-dot')).toBeTruthy();
    needsYou().markAllRead();
    await waitFor(() => expect(screen.queryByTestId('shell-needs-you-dot')).toBeNull());
    expect(bell.getAttribute('aria-label')).toBe('Notifications');
  });
});
