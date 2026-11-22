// @vitest-environment jsdom
/**
 * The settings model field opens the ONE model dialog the chat / New-chat
 * chips open (OIN-39): titled by the field's label — Now line, search over
 * every model, Single / Fusion tabs (Fusion off: a field takes one model),
 * Cycle set + ⋯ row menu, "Use at startup", "Use default". The list needs no
 * open chat: it comes from the PC's own registry.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { createSignal } from 'solid-js';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { hostCmd } from '../../../settings/hostCmd';
import { z } from 'zod';
import { setHostClientFactory } from '../../../lib/host/factory';
import { resetHubs } from '../../../lib/session/hub';
import { resetSessions, sessions } from '../../../lib/session/store';
import { resetLayout, setDesktopOverride } from '../../../lib/layout';
import { MOCK_MODELS } from '../../../dev/mock/models';
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

/** A PC with NO chat open: only the host's own registry can answer. */
async function bootNoChat(name = 'model-draft-fusion') {
  const scenario = SCENARIOS[name]!;
  const world = createMockHost({ ...scenario, hosts: scenario.hosts.map((host) => ({ ...host, models: MOCK_MODELS })), speed: 0 });
  setHostClientFactory(world.client);
  await tick();
  expect(sessions().keys()).toHaveLength(0);
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
  it('opens the shared model dialog titled by the label; a row picks its key and closes, "Use default" (footer) clears', async () => {
    await boot();
    const onPick = vi.fn();
    render(() => <ModelPickerField hostId={HOST} label="Recap model" value="" fallback="Session model" testId="f" onPick={onPick} />);
    expect(screen.getByTestId('f').textContent).toContain('Session model');
    fireEvent.click(screen.getByTestId('f'));
    const pop = await screen.findByTestId('model-pop-f');
    expect(pop.getAttribute('role')).toBe('dialog');
    expect(pop.querySelector('h2')?.textContent).toBe('Recap model');
    expect(pop.querySelector('[data-testid="model-search"]')).toBeTruthy();
    expect(screen.getByTestId('model-picker-now').textContent).toContain('Default — Session model');
    await waitFor(() => expect(screen.getByTestId('model-picker-list').textContent).toContain('GPT-5'));
    // The full dialog: the Cycle set filter and "Use at startup" are there too.
    expect(screen.getByTestId('model-filter-cycle')).toBeTruthy();
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

  it('lists every model with NO chat open (the host registry), no "open a chat" empty state; search filters', async () => {
    await bootNoChat();
    render(() => <ModelPickerField hostId={HOST} label="Decision model" value="" fallback="Inherit" testId="f" onPick={() => {}} />);
    fireEvent.click(screen.getByTestId('f'));
    const pop = await screen.findByTestId('model-pop-f');
    await waitFor(() => expect(screen.getByTestId('model-picker-list').textContent).toContain('Claude'));
    expect(pop.textContent).not.toContain('once a chat is open');
    expect(screen.getByTestId('model-search').getAttribute('placeholder')).toMatch(/^Search all \d+ models$/);
    const all = screen.getByTestId('model-picker-list').querySelectorAll('[data-key]').length;
    expect(all).toBeGreaterThan(1);
    fireEvent.input(screen.getByTestId('model-search'), { target: { value: 'glm' } });
    await waitFor(() => {
      const rows = [...screen.getByTestId('model-picker-list').querySelectorAll('[data-key]')];
      expect(rows.length).toBeGreaterThan(0);
      expect(rows.length).toBeLessThan(all);
      expect(rows.every((r) => /glm/i.test(`${r.getAttribute('data-key')} ${r.textContent}`))).toBe(true);
    });
  });

  it('a pick saves the field and the row reflects it (name · provider); "Use default" clears it', async () => {
    await bootNoChat();
    const [value, setValue] = createSignal('');
    render(() => <ModelPickerField hostId={HOST} label="Decision model" value={value()} fallback="Inherit" testId="f" onPick={(k) => setValue(k ?? '')} />);
    fireEvent.click(screen.getByTestId('f'));
    await waitFor(() => expect(screen.getByTestId('model-picker-list').querySelector('[data-key="anthropic/claude-sonnet-4-5"]')).toBeTruthy());
    fireEvent.click(screen.getByTestId('model-picker-list').querySelector('[data-key="anthropic/claude-sonnet-4-5"]')!);
    expect(value()).toBe('anthropic/claude-sonnet-4-5');
    await waitFor(() => expect(screen.queryByTestId('model-pop-f')).toBeNull());
    await waitFor(() => expect(screen.getByTestId('f').querySelector('.settings-model-name')?.textContent).toMatch(/Sonnet/));
    expect(screen.getByTestId('f').querySelector('.settings-model-prov')?.textContent).toBe('anthropic');
    fireEvent.click(screen.getByTestId('f'));
    await waitFor(() => expect(screen.getByTestId('model-picker-now').textContent).toMatch(/Sonnet.* · anthropic/));
    fireEvent.click(await screen.findByTestId('model-use-default'));
    expect(value()).toBe('');
    await waitFor(() => expect(screen.getByTestId('f').textContent).toContain('Inherit'));
  });

  it('shows the Fusion tab disabled with the single-model hint when Fusion is on; absent when it is off', async () => {
    await bootNoChat();
    render(() => <ModelPickerField hostId={HOST} label="Decision model" value="" fallback="Inherit" testId="f" onPick={() => {}} />);
    fireEvent.click(screen.getByTestId('f'));
    const tab = await screen.findByTestId('model-tab-fusion');
    expect((tab as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByTestId('model-fusion-off').textContent).toBe('This setting takes a single model');
    expect(screen.getByTestId('model-tab-single').getAttribute('aria-selected')).toBe('true');
    cleanup();
    resetSessions();
    resetHubs();
    await bootNoChat('model-picker');
    render(() => <ModelPickerField hostId={HOST} label="Decision model" value="" fallback="Inherit" testId="g" onPick={() => {}} />);
    fireEvent.click(screen.getByTestId('g'));
    await screen.findByTestId('model-pop-g');
    await waitFor(() => expect(screen.getByTestId('model-picker-list').textContent).toContain('GPT-5'));
    expect(screen.queryByTestId('model-tab-fusion')).toBeNull();
  });

  it('the cycle set and "Use at startup" keep their GLOBAL meaning from Settings', async () => {
    await bootNoChat();
    const [value, setValue] = createSignal('anthropic/claude-sonnet-4-5');
    render(() => <ModelPickerField hostId={HOST} label="Decision model" value={value()} fallback="Inherit" testId="f" onPick={(k) => setValue(k ?? '')} />);
    fireEvent.click(screen.getByTestId('f'));
    const startup = await screen.findByTestId('model-use-startup');
    await waitFor(() => expect((startup as HTMLButtonElement).disabled).toBe(false));
    fireEvent.click(startup);
    await waitFor(async () => {
      const res = await hostCmd({ kind: 'get_settings', key: 'defaultModel' }, z.object({ value: z.unknown() }), HOST);
      expect(res.ok && res.value.value).toBe('claude-sonnet-4-5');
    });
    // The dialog stays open (a global write, not a pick) and the field is unchanged.
    expect(screen.getByTestId('model-pop-f')).toBeTruthy();
    expect(value()).toBe('anthropic/claude-sonnet-4-5');
    // The cycle set toggles through the row's ⋯ menu.
    expect(screen.getByTestId('model-filter-cycle')).toBeTruthy();
  });

  it('typing provider/model is only a collapsed "Advanced" escape hatch at the bottom', async () => {
    await bootNoChat();
    const onPick = vi.fn();
    render(() => <ModelPickerField hostId={HOST} label="Decision model" value="" fallback="Inherit" testId="f" onPick={onPick} />);
    fireEvent.click(screen.getByTestId('f'));
    await waitFor(() => expect(screen.getByTestId('model-picker-list')).toBeTruthy());
    const adv = screen.getByTestId('host-model-advanced') as HTMLDetailsElement;
    expect(adv.open).toBe(false);
    expect(adv.compareDocumentPosition(screen.getByTestId('model-picker-list')) & Node.DOCUMENT_POSITION_PRECEDING).toBeTruthy();
    adv.open = true;
    const input = screen.getByTestId('host-model-advanced-input') as HTMLInputElement;
    fireEvent.input(input, { target: { value: 'acme/custom-1' } });
    fireEvent.submit(input.closest('form')!);
    expect(onPick).toHaveBeenLastCalledWith('acme/custom-1');
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
