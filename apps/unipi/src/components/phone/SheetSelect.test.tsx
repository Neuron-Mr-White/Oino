// @vitest-environment jsdom
import { createSignal } from 'solid-js';
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { SheetFrame, inFloatingLayer } from './Sheet';
import { EnumSelect } from '../../ui/settings/fields';

afterEach(() => { cleanup(); closes = 0; });

let closes = 0;
function Harness() {
  const [open, setOpen] = createSignal(true);
  const [v, setV] = createSignal('off');
  return (
    <SheetFrame open={open()} onClose={() => { closes++; setOpen(false); }} label="Edit agent" testId="sheet">
      <EnumSelect
        field={{ key: 'thinking', label: 'Thinking', options: [{ value: 'off', label: 'Off' }, { value: 'high', label: 'High' }] } as never}
        value={v()}
        disabled={false}
        onChange={setV}
      />
    </SheetFrame>
  );
}

const pointer = (el: Element) => {
  fireEvent.pointerDown(el, { pointerType: 'mouse', button: 0 });
  fireEvent.pointerUp(el, { pointerType: 'mouse', button: 0 });
  fireEvent.click(el);
};

describe('SheetFrame + portaled Select', () => {
  it('subagents_thinking_select_keeps_sheet_open', async () => {
    render(() => <Harness />);
    const trigger = await screen.findByTestId('set-thinking');
    fireEvent.pointerDown(trigger, { pointerType: 'mouse', button: 0 });
    fireEvent.click(trigger);
    const option = await screen.findByRole('option', { name: /High/ });
    pointer(option);
    await waitFor(() => expect(screen.getByTestId('set-thinking').textContent).toContain('High'));
    expect(closes).toBe(0);
    expect(screen.queryByTestId('sheet')).not.toBeNull();
  });

  it('a scrim tap still closes', async () => {
    const { container } = render(() => <Harness />);
    await screen.findByTestId('sheet');
    const scrim = document.querySelector('.ph-scrim')!;
    pointer(scrim);
    pointer(document.body);
    await waitFor(() => expect(screen.queryByTestId('sheet')).toBeNull());
    void container;
  });

  it('inFloatingLayer detects portaled listboxes only', () => {
    const el = document.createElement('div');
    el.setAttribute('data-popper-positioner', '');
    const child = document.createElement('span');
    el.append(child);
    expect(inFloatingLayer(child)).toBe(true);
    expect(inFloatingLayer(document.body)).toBe(false);
    expect(inFloatingLayer(null)).toBe(false);
  });

  it('a dialog mounted outside the sheet keeps it open', async () => {
    render(() => <Harness />);
    await screen.findByTestId('sheet');
    // A dialog in its own root (a portal), above the sheet.
    const host = document.createElement('div');
    document.body.appendChild(host);
    render(() => (
      <div role="dialog" aria-modal="true" data-testid="outer-dialog">
        <button type="button" data-testid="outer-btn">pick</button>
      </div>
    ), { container: host });
    pointer(await screen.findByTestId('outer-btn'));
    expect(closes).toBe(0);
    expect(screen.queryByTestId('sheet')).not.toBeNull();
    host.remove();
  });
});
