// @vitest-environment jsdom
/**
 * The shared modal layer (owner 2026-11-06): picking something inside a
 * portaled child — a Kobalte select, the model dialog — must never dismiss
 * the sheet below it. The model dialog is the app's own overlay; the select is
 * Kobalte's (mounted outside the sheet's tree).
 */
import { createSignal } from 'solid-js';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { SheetFrame } from './Sheet';
import { ModelPickerField } from '../../ui/settings/ModelPickerField';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { SCENARIOS } from '../../dev/mock/scenarios';
import { sessions, resetSessions } from '../../lib/session/store';
import { setDesktopOverride, resetLayout } from '../../lib/layout';

afterEach(() => {
  cleanup();
  resetSessions();
  resetLayout();
  setHostClientFactory(undefined);
  closes = 0;
});

let closes = 0;
function Harness(props: { children: unknown }) {
  const [open, setOpen] = createSignal(true);
  return (
    <SheetFrame open={open()} onClose={() => { closes++; setOpen(false); }} label="Edit agent" testId="sheet">
      {props.children as never}
    </SheetFrame>
  );
}

const pointer = (el: Element) => {
  fireEvent.pointerDown(el, { pointerType: 'mouse', button: 0 });
  fireEvent.pointerUp(el, { pointerType: 'mouse', button: 0 });
  fireEvent.click(el);
};

describe('a portaled dialog inside a sheet', () => {
  it('picking a model keeps the sheet open', async () => {
    setDesktopOverride(true);
    const scenario = SCENARIOS['model-picker']!;
    const world = createMockHost({ ...scenario, speed: 0 });
    setHostClientFactory(world.client);
    await new Promise((r) => setTimeout(r, 0));
    const s = sessions().ensure('this-pc', 's001');
    await waitFor(() => expect(s.state.conn).toBe('live'));
    render(() => (
      <Harness>
        <ModelPickerField hostId="this-pc" label="Model" value="" fallback="Inherit" testId="agent-model" onPick={() => {}} />
      </Harness>
    ));
    fireEvent.click(await screen.findByTestId('agent-model'));
    const pop = await screen.findByTestId('model-pop-agent-model');
    await waitFor(() => expect(pop.querySelector('[data-key="openai/gpt-5"]')).toBeTruthy());
    pointer(pop.querySelector('[data-key="openai/gpt-5"]')!);
    await waitFor(() => expect(screen.queryByTestId('model-pop-agent-model')).toBeNull());
    // The dialog closed, the sheet below it did not.
    expect(closes).toBe(0);
    expect(screen.queryByTestId('sheet')).not.toBeNull();
  });

  it('the dialog’s own backdrop closes the dialog, not the sheet', async () => {
    setDesktopOverride(true);
    const scenario = SCENARIOS['model-picker']!;
    const world = createMockHost({ ...scenario, speed: 0 });
    setHostClientFactory(world.client);
    await new Promise((r) => setTimeout(r, 0));
    const s = sessions().ensure('this-pc', 's001');
    await waitFor(() => expect(s.state.conn).toBe('live'));
    render(() => (
      <Harness>
        <ModelPickerField hostId="this-pc" label="Model" value="" fallback="Inherit" testId="agent-model" onPick={() => {}} />
      </Harness>
    ));
    fireEvent.click(await screen.findByTestId('agent-model'));
    const overlay = await screen.findByTestId('model-picker-overlay');
    pointer(overlay);
    await waitFor(() => expect(screen.queryByTestId('model-pop-agent-model')).toBeNull());
    expect(closes).toBe(0);
    expect(screen.queryByTestId('sheet')).not.toBeNull();
  });
});
