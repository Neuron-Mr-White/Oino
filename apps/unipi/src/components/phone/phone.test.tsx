// @vitest-environment jsdom
/** Phone primitives (phone spec §2.12): pure helpers + the controls' a11y contract. */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { Plus } from 'lucide-solid';
import { keyboardOverlap, hapticKind, setTabBarHidden, tabBarHiddenByLayer } from './services';
import { snapPointsFor, sizeFor } from './Sheet';
import { swipeTriggered } from './lists';
import { IconButton, ListRow, Segmented, Stepper, Switch } from './index';

afterEach(() => cleanup());

describe('keyboardOverlap (--kb, §2.10)', () => {
  it('native inset with no resize = the whole keyboard', () => expect(keyboardOverlap(300, 900, 900, 0)).toBe(300));
  it('subtracts what Android already resized (no double inset, PH-06)', () => expect(keyboardOverlap(300, 900, 700, 0)).toBe(100));
  it('fully resized = 0', () => expect(keyboardOverlap(300, 900, 600, 0)).toBe(0));
  it('browser pan', () => expect(keyboardOverlap(0, 900, 900, 280)).toBe(280));
  it('ignores tiny insets', () => expect(keyboardOverlap(24, 900, 900, 0)).toBe(0));
});

describe('sheets', () => {
  it('detents → snap points / size', () => {
    expect(snapPointsFor(['content'])).toEqual([0, 1]);
    expect(snapPointsFor(['half', 'full'])).toEqual([0, 0.5, 1]);
    expect(sizeFor(['half', 'full'])).toBe('full');
    expect(sizeFor(['half'])).toBe('half');
    expect(sizeFor(['content'])).toBe('content');
  });
});

describe('services', () => {
  it('haptic kinds', () => {
    expect(hapticKind('selection')).toBe('tick');
    expect(hapticKind('impact')).toBe('tap');
  });
  it('tab bar reasons', () => {
    setTabBarHidden('a', true);
    setTabBarHidden('b', true);
    setTabBarHidden('a', false);
    expect(tabBarHiddenByLayer()).toBe(true);
    setTabBarHidden('b', false);
    expect(tabBarHiddenByLayer()).toBe(false);
  });
  it('swipe threshold', () => {
    expect(swipeTriggered(140, 400)).toBe(true);
    expect(swipeTriggered(-100, 400)).toBe(false);
  });
});

describe('controls', () => {
  it('IconButton is labelled and shows a capped badge', () => {
    const click = vi.fn();
    render(() => <IconButton icon={Plus} label="New chat" badge={12} onClick={click} />);
    const btn = screen.getByRole('button', { name: 'New chat' });
    expect(btn.textContent).toContain('9+');
    fireEvent.click(btn);
    expect(click).toHaveBeenCalled();
  });
  it('Switch is role=switch and toggles', () => {
    const change = vi.fn();
    render(() => <Switch checked={false} label="Haptics" onChange={change} />);
    const sw = screen.getByRole('switch', { name: 'Haptics' });
    expect(sw.getAttribute('aria-checked')).toBe('false');
    fireEvent.click(sw);
    expect(change).toHaveBeenCalledWith(true);
  });
  it('Segmented / Stepper', () => {
    const seg = vi.fn();
    const step = vi.fn();
    render(() => (
      <>
        <Segmented label="Thinking" value="off" options={[{ value: 'off', label: 'Off' }, { value: 'high', label: 'High' }] as const} onChange={seg} />
        <Stepper label="Size" value={5} min={1} max={5} onChange={step} />
      </>
    ));
    fireEvent.click(screen.getByRole('radio', { name: 'High' }));
    expect(seg).toHaveBeenCalledWith('high');
    expect((screen.getByRole('button', { name: 'Increase Size' }) as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(screen.getByRole('button', { name: 'Decrease Size' }));
    expect(step).toHaveBeenCalledWith(4);
  });
  it('ListRow: click, and a contextmenu long-press does not also click', () => {
    const click = vi.fn();
    const long = vi.fn();
    render(() => <ListRow title="Chat A" subtitle="proj · 2m" onClick={click} onLongPress={long} testId="row" />);
    const row = screen.getByTestId('row');
    fireEvent.click(row);
    expect(click).toHaveBeenCalledTimes(1);
    fireEvent.contextMenu(row);
    expect(long).toHaveBeenCalledTimes(1);
    fireEvent.click(row);
    expect(click).toHaveBeenCalledTimes(1);
  });
});
