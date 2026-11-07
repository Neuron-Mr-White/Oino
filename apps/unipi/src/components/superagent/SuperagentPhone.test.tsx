// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { superagentLeads } from '../../dev/mock/scenarios/superagent-leads';
import { Root } from '../../main';
import { resetSessions } from '../../lib/session/store';
import { resetHubs } from '../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); setHostClientFactory(undefined); resetSessions(); resetHubs(); resetLayout(); setDesktopOverride(undefined); setPlatformOverride(undefined); });

describe('Superagent on the phone', () => {
  it('keeps chat first with a Needs you badge, and opens the sheet in one tap', async () => {
    setHostClientFactory(createMockHost(superagentLeads).client);
    setPlatformOverride('tauri-desktop'); setDesktopOverride(false);
    location.hash = '#/superagent?session=superagent-chat&host=pc-coffee';
    render(() => <Root />);
    const button = await screen.findByTestId('sa-sheet-open');
    // The pane is closed: the badge comes from the page-level status poll.
    await waitFor(() => expect(button.getAttribute('aria-label')).toBe('Needs you · 5'));
    expect(button.textContent).toContain('5');
    expect(screen.queryByTestId('sa-needs')).toBeNull();
    fireEvent.click(button);
    await screen.findByTestId('sa-needs');
    expect(screen.getByRole('radio', { name: 'Needs you · 5' }).getAttribute('aria-checked')).toBe('true');
    expect(screen.getByRole('radio', { name: 'Leads · 2' })).toBeTruthy();
    expect(screen.getByRole('radio', { name: 'Projects' })).toBeTruthy();
  });
});
