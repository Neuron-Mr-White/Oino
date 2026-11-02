// @vitest-environment jsdom
/**
 * The settings model field opens the ONE model dialog the chat / New-chat
 * chips open: titled by the field's label, the shared search and list
 * (readiness, display names) — with "Use default" in its footer and no
 * Fusion tab, Cycle set or "Use at startup".
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
  it('opens the shared model dialog titled by the label; a row picks its key, "Use default" (footer) clears', async () => {
    await boot();
    const onPick = vi.fn();
    render(() => <ModelPickerField hostId={HOST} label="Recap model" value="" fallback="Session model" testId="f" onPick={onPick} />);
    expect(screen.getByTestId('f').textContent).toContain('Session model');
    fireEvent.click(screen.getByTestId('f'));
    const pop = await screen.findByTestId('model-pop-f');
    expect(pop.getAttribute('role')).toBe('dialog');
    expect(pop.querySelector('h2')?.textContent).toBe('Recap model');
    expect(pop.querySelector('[data-testid="model-search"]')).toBeTruthy();
    await waitFor(() => expect(screen.getByTestId('model-picker-list').textContent).toContain('GPT-5'));
    expect(screen.queryByTestId('model-tab-fusion')).toBeNull();
    expect(screen.queryByTestId('model-picker-cycle-toggle')).toBeNull();
    expect(screen.queryByTestId('model-use-startup')).toBeNull();
    const useDefault = screen.getByTestId('model-use-default');
    expect(screen.getByTestId('model-picker-foot').contains(useDefault)).toBe(true);
    expect(useDefault.getAttribute('aria-pressed')).toBe('true');
    fireEvent.click(screen.getByTestId('model-picker-list').querySelector('[data-key="openai/gpt-5"]')!);
    expect(onPick).toHaveBeenLastCalledWith('openai/gpt-5');
    await waitFor(() => expect(screen.queryByTestId('model-pop-f')).toBeNull());
    fireEvent.click(screen.getByTestId('f'));
    fireEvent.click(await screen.findByTestId('model-use-default'));
    expect(onPick).toHaveBeenLastCalledWith(undefined);
  });

  it('the field shows the display name + provider; the dialog\'s Now line does too, never cut', async () => {
    await boot();
    render(() => <ModelPickerField hostId={HOST} label="Default model" value="openai/gpt-5" fallback="Session model" testId="f" onPick={() => {}} />);
    await waitFor(() => expect(screen.getByTestId('f').querySelector('.settings-model-name')?.textContent).toBe('GPT-5'));
    expect(screen.getByTestId('f').querySelector('.settings-model-prov')?.textContent).toBe('openai');
    fireEvent.click(screen.getByTestId('f'));
    await waitFor(() => expect(screen.getByTestId('model-picker-now').textContent).toContain('GPT-5 · openai'));
    expect(screen.getByTestId('model-use-default').getAttribute('aria-pressed')).toBe('false');
    // focus returns to the field on close
    fireEvent.click(screen.getByTestId('model-picker-close'));
    await waitFor(() => expect(screen.queryByTestId('model-pop-f')).toBeNull());
    expect(document.activeElement).toBe(screen.getByTestId('f'));
  });
});
