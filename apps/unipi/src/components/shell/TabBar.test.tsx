/** The phone tab bar's Superagent centrepiece: glow class always, needs-you modifier + pulse when it needs the owner. */
import { cleanup, render, screen } from '@solidjs/testing-library';
import { MemoryRouter, Route } from '@solidjs/router';
import { createSignal } from 'solid-js';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { RailSummary } from '../superagent/railStatus';

const [summary, setSummary] = createSignal<RailSummary | undefined>();
vi.mock('../superagent/railStatus', () => ({ useRailStatus: () => summary }));
vi.mock('../superagent/chatActivity', () => ({ useSuperagentChatWorking: () => () => false }));
vi.mock('./HostSwitcher', () => ({ useSelectedHost: () => () => ({ id: 'h1' }) }));
vi.mock('../../lib/platform', () => ({ can: () => true }));
vi.mock('../../ui/slots/reserved', () => ({ reservedGate: () => true }));
vi.mock('../phone/services', () => ({ haptic: () => {} }));

import { TabBar } from './TabBar';

function mount() {
  return render(() => (
    <MemoryRouter root={() => <TabBar />}>
      <Route path="/*" component={() => null} />
    </MemoryRouter>
  ));
}

afterEach(() => {
  cleanup();
  setSummary(undefined);
});

describe('TabBar Superagent glow', () => {
  it('renders the Superagent tab with the glow class and no needs-you modifier by default', () => {
    mount();
    const tab = screen.getByTestId('tab-superagent');
    expect(tab.classList.contains('ph-tab-hero')).toBe(true);
    expect(tab.classList.contains('ph-tab-glow')).toBe(true);
    expect(tab.classList.contains('ph-tab-needs')).toBe(false);
    expect(screen.queryByTestId('tab-superagent-pulse')).toBeNull();
    expect(screen.getByTestId('tab-chat').classList.contains('ph-tab-glow')).toBe(false);
  });

  it('gets the needs-you modifier, a pulse and a label when Superagent needs the owner', () => {
    setSummary({ needs: 2, leads: 1 });
    mount();
    const tab = screen.getByTestId('tab-superagent');
    expect(tab.classList.contains('ph-tab-glow')).toBe(true);
    expect(tab.classList.contains('ph-tab-needs')).toBe(true);
    expect(screen.getByTestId('tab-superagent-pulse')).toBeTruthy();
    expect(tab.getAttribute('aria-label')).toContain('needs you');
  });

  it('drops the modifier again once nothing needs the owner', () => {
    setSummary({ needs: 1, leads: 0 });
    mount();
    const tab = screen.getByTestId('tab-superagent');
    expect(tab.classList.contains('ph-tab-needs')).toBe(true);
    setSummary({ needs: 0, leads: 0 });
    expect(tab.classList.contains('ph-tab-needs')).toBe(false);
    expect(screen.queryByTestId('tab-superagent-pulse')).toBeNull();
  });
});
