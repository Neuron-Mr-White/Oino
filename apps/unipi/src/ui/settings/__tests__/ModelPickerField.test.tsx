// @vitest-environment jsdom
/**
 * The settings model field opens the ONE model popover (`ModelPopover`) the
 * chat / New-chat chips open: the shared search and list (readiness,
 * display names), anchored to the field — with "Use default" as its footer row.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { setHostClientFactory } from '../../../lib/host/factory';
import { resetHubs } from '../../../lib/session/hub';
import { resetSessions, sessions } from '../../../lib/session/store';
import { resetLayout, setDesktopOverride } from '../../../lib/layout';
import { createMockHost } from '../../../dev/mock/host';
import { SCENARIOS } from '../../../dev/mock/scenarios';
import { ModelPickerField } from '../ModelPickerField';

const HOST = 'this-pc';
const tick = () => new Promise((r) => setTimeout(r, 0));

async function boot() {
  const scenario = SCENARIOS['model-picker']!;
  const world = createMockHost({ ...scenario, speed: 0 });
  setHostClientFactory(world.client);
  await tick();
  const s = sessions().ensure(HOST, 's001');
  await waitFor(() => expect(s.state.conn).toBe('live'));
  return world;
}

beforeEach(() => setDesktopOverride(true));
afterEach(() => {
  cleanup();
  resetSessions();
  resetHubs();
  resetLayout();
  setHostClientFactory(undefined);
  window.localStorage.clear();
});

describe('ModelPickerField', () => {
  it('opens the shared model popover; a row picks its key, "Use default" (footer) clears', async () => {
    await boot();
    const onPick = vi.fn();
    render(() => <ModelPickerField hostId={HOST} label="Model" value="" fallback="Session model" testId="f" onPick={onPick} />);
    expect(screen.getByTestId('f').textContent).toContain('Session model');
    fireEvent.click(screen.getByTestId('f'));
    const pop = await screen.findByTestId('model-pop-f');
    expect(pop.closest('[role="menu"]')).toBeTruthy();
    expect(pop.querySelector('[data-testid="model-search"]')).toBeTruthy();
    await waitFor(() => expect(screen.getByTestId('model-picker-list').textContent).toContain('GPT-5'));
    const useDefault = screen.getByTestId('model-use-default');
    expect(screen.getByTestId('model-picker-foot').contains(useDefault)).toBe(true);
    expect(useDefault.getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByTestId('model-picker-list').querySelector('[data-key="openai/gpt-5"]')!);
    expect(onPick).toHaveBeenLastCalledWith('openai/gpt-5');
    await waitFor(() => expect(screen.queryByTestId('model-pop-f')).toBeNull());
    fireEvent.click(screen.getByTestId('f'));
    fireEvent.click(await screen.findByTestId('model-use-default'));
    expect(onPick).toHaveBeenLastCalledWith(undefined);
  });
});
