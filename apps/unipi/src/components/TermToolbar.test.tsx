// @vitest-environment jsdom
import { render, screen, fireEvent, cleanup } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import TermToolbar from './TermToolbar';
import { Modifiers } from '../lib/termToolbar';
import { DEFAULT_TOOLBAR_KEYS } from '../settings/schema';

vi.mock('@tauri-apps/plugin-haptics', () => ({ impactFeedback: vi.fn().mockResolvedValue(undefined) }));
afterEach(cleanup);

function mount(over: Partial<Parameters<typeof TermToolbar>[0]> = {}) {
  const bytes: string[] = [];
  const modifiers = new Modifiers();
  const props = {
    keys: DEFAULT_TOOLBAR_KEYS,
    haptics: false,
    modifierIcons: false,
    keyboardOpen: false,
    modifiers,
    onBytes: (b: string) => bytes.push(b),
    onPaste: vi.fn(),
    onShortcuts: vi.fn(),
    onHistory: vi.fn(),
    onComposer: vi.fn(),
    onKeyboardToggle: vi.fn(),
    ...over,
  };
  render(() => <TermToolbar {...props} />);
  return { bytes, modifiers, props };
}

describe('TermToolbar', () => {
  it('text labels by default, in Moshi order, with function aria-labels', () => {
    mount();
    const labels = [...document.querySelectorAll('.term-key-scroll .term-key')].map((el) => el.textContent?.trim());
    expect(labels.slice(0, 5)).toEqual(['Ctrl', 'Shift', 'Esc', 'Tab', 'Opt']);
    for (const name of ['Ctrl modifier', 'Shift modifier', 'Escape', 'Tab', 'Option (Meta) modifier', 'Shortcuts', 'Command history', 'Composer', 'Show keyboard']) {
      expect(screen.getByRole(name.endsWith('modifier') ? 'button' : 'button', { name: new RegExp(`^${name.replace(/[()]/g, '\\$&')}`) })).toBeInTheDocument();
    }
    expect(screen.getByRole('slider', { name: /Arrow keys/ })).toBeInTheDocument();
  });

  it('icon mode shows no text labels for the modifier row', () => {
    mount({ modifierIcons: true });
    const first = document.querySelector('.term-key-scroll .term-key');
    expect(first?.textContent?.trim()).toBe('');
    expect(first?.querySelector('svg')).not.toBeNull();
  });

  it('keys never take focus (mousedown default prevented)', () => {
    mount();
    for (const el of document.querySelectorAll('.term-key')) {
      const event = new MouseEvent('mousedown', { bubbles: true, cancelable: true });
      el.dispatchEvent(event);
      expect(event.defaultPrevented).toBe(true);
    }
  });

  it('modifier tap arms, second tap disarms; armed Shift turns Tab into back-tab', () => {
    const { bytes, modifiers } = mount();
    const shift = screen.getByRole('button', { name: /^Shift modifier/ });
    fireEvent.click(shift);
    expect(modifiers.isActive('shift')).toBe(true);
    vi.useFakeTimers();
    vi.advanceTimersByTime(1000);
    fireEvent.click(shift);
    vi.useRealTimers();
    expect(modifiers.isActive('shift')).toBe(false);
    fireEvent.click(shift);
    fireEvent.click(screen.getByRole('button', { name: 'Tab' }));
    expect(bytes).toEqual(['\x1b[Z']);
    expect(modifiers.isActive('shift')).toBe(false);
  });

  it('keyboard key is lit while the keyboard is up', () => {
    const { props } = mount({ keyboardOpen: true });
    const key = screen.getByRole('button', { name: 'Hide keyboard' });
    expect(key).toHaveClass('term-key-lit');
    expect(key).toHaveAttribute('aria-pressed', 'true');
    fireEvent.click(key);
    expect(props.onKeyboardToggle).toHaveBeenCalled();
  });

  it('arrow pad: tap sends nothing; a drag sends one arrow, then auto-repeats after a delay until release', () => {
    vi.useFakeTimers();
    try {
      const { bytes, modifiers } = mount();
      const pad = screen.getByRole('slider', { name: /Arrow keys/ });
      fireEvent.pointerDown(pad, { clientX: 100, clientY: 100, pointerId: 1 });
      fireEvent.pointerUp(pad, { clientX: 101, clientY: 100, pointerId: 1 });
      vi.advanceTimersByTime(2000);
      expect(bytes).toEqual([]);

      fireEvent.pointerDown(pad, { clientX: 100, clientY: 100, pointerId: 1 });
      fireEvent.pointerMove(pad, { clientX: 100, clientY: 86, pointerId: 1 });
      expect(bytes).toEqual(['\x1b[A']);
      expect(pad).toHaveAttribute('data-dir', 'up');
      // Dragging further does not flood: still one arrow.
      fireEvent.pointerMove(pad, { clientX: 100, clientY: 20, pointerId: 1 });
      fireEvent.pointerMove(pad, { clientX: 100, clientY: -80, pointerId: 1 });
      expect(bytes).toHaveLength(1);
      // Held: nothing during the initial delay, then a steady repeat.
      vi.advanceTimersByTime(399);
      expect(bytes).toHaveLength(1);
      vi.advanceTimersByTime(1);
      expect(bytes).toHaveLength(2);
      vi.advanceTimersByTime(100);
      expect(bytes).toHaveLength(3);
      vi.advanceTimersByTime(1000);
      const held = bytes.length;
      expect(held).toBeGreaterThan(10);
      expect(held).toBeLessThan(20);
      expect(new Set(bytes)).toEqual(new Set(['\x1b[A']));

      // Turn right with Ctrl armed: one modified arrow, delay restarts.
      modifiers.tap('ctrl', 0);
      fireEvent.pointerMove(pad, { clientX: 160, clientY: 95, pointerId: 1 });
      expect(bytes.at(-1)).toBe('\x1b[1;5C');
      expect(bytes).toHaveLength(held + 1);
      vi.advanceTimersByTime(399);
      expect(bytes).toHaveLength(held + 1);
      vi.advanceTimersByTime(1);
      expect(bytes.at(-1)).toBe('\x1b[C');

      // Release stops the repeat.
      fireEvent.pointerUp(pad, { pointerId: 1 });
      const released = bytes.length;
      vi.advanceTimersByTime(2000);
      expect(bytes).toHaveLength(released);
      expect(pad).not.toHaveAttribute('data-dir');
    } finally {
      vi.useRealTimers();
    }
  });

  it('arrow pad: back to the centre stops the repeat without lifting', () => {
    vi.useFakeTimers();
    try {
      const { bytes } = mount();
      const pad = screen.getByRole('slider', { name: /Arrow keys/ });
      fireEvent.pointerDown(pad, { clientX: 100, clientY: 100, pointerId: 1 });
      fireEvent.pointerMove(pad, { clientX: 100, clientY: 130, pointerId: 1 });
      expect(bytes).toEqual(['\x1b[B']);
      fireEvent.pointerMove(pad, { clientX: 100, clientY: 102, pointerId: 1 });
      vi.advanceTimersByTime(2000);
      expect(bytes).toEqual(['\x1b[B']);
      expect(pad).not.toHaveAttribute('data-dir');
    } finally {
      vi.useRealTimers();
    }
  });
});
