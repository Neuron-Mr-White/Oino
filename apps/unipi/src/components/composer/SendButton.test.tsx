// @vitest-environment jsdom
/**
 * SendButton (WP-A11, Z-52 + Z-55): Send ↔ Stop, the chevron menu's
 * follow-up default radios, "Send now (stops run)", and the popup Esc rule
 * (Esc closes the menu and is consumed — it never stops the run).
 */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { followUpMode } from '../../lib/composer/sendMode';
import { SendButton } from './SendButton';

beforeEach(() => localStorage.clear());
afterEach(() => {
  cleanup();
  localStorage.clear();
});

const open = () => fireEvent.click(screen.getByTestId('send-chevron'));

describe('SendButton', () => {
  it('is Send when idle and Stop while running (Z-55)', () => {
    render(() => <SendButton running={false} onSend={() => {}} onStop={() => {}} />);
    expect(screen.getByTestId('send-send')).toBeTruthy();
    expect(screen.queryByTestId('send-stop')).toBeNull();
    cleanup();
    render(() => <SendButton running empty onSend={() => {}} onStop={() => {}} />);
    expect(screen.getByTestId('send-stop')).toBeTruthy();
    expect(screen.queryByTestId('send-send')).toBeNull();
  });

  it('offers Send and Stop together for a follow-up', () => {
    const onSend = vi.fn();
    const onStop = vi.fn();
    render(() => <SendButton running empty={false} onSend={onSend} onStop={onStop} />);
    fireEvent.click(screen.getByTestId('send-send'));
    fireEvent.click(screen.getByTestId('send-stop'));
    expect(onSend).toHaveBeenCalledOnce();
    expect(onStop).toHaveBeenCalledOnce();
    expect(screen.getByTestId('send-chevron')).toBeTruthy();
  });

  it('sends and stops', () => {
    const onSend = vi.fn();
    const onStop = vi.fn();
    render(() => <SendButton running={false} onSend={onSend} onStop={onStop} />);
    fireEvent.click(screen.getByTestId('send-send'));
    expect(onSend).toHaveBeenCalledTimes(1);
    cleanup();
    render(() => <SendButton running onSend={onSend} onStop={onStop} />);
    fireEvent.click(screen.getByTestId('send-stop'));
    expect(onStop).toHaveBeenCalledTimes(1);
  });

  it('Send is disabled while empty; Stop never is', () => {
    render(() => <SendButton running={false} empty onSend={() => {}} onStop={() => {}} />);
    expect((screen.getByTestId('send-send') as HTMLButtonElement).disabled).toBe(true);
    cleanup();
    render(() => <SendButton running empty onSend={() => {}} onStop={() => {}} />);
    expect((screen.getByTestId('send-stop') as HTMLButtonElement).disabled).toBe(false);
  });

  it('the menu switches the global follow-up default (queue | steer)', () => {
    render(() => <SendButton running={false} onSend={() => {}} onStop={() => {}} />);
    open();
    expect(screen.getByTestId('send-mode-queue').getAttribute('aria-checked')).toBe('true');
    fireEvent.click(screen.getByTestId('send-mode-steer'));
    expect(followUpMode()).toBe('steer');
    expect(localStorage.getItem('unipi:composer:followUpMode')).toBe('steer');
    // Choosing an item closes the menu; reopening shows the new default.
    open();
    expect(screen.getByTestId('send-mode-steer').getAttribute('aria-checked')).toBe('true');
  });

  it('"Send now (stops run)" appears only while running and calls onSendNow', () => {
    render(() => <SendButton running={false} onSend={() => {}} onStop={() => {}} />);
    open();
    expect(screen.queryByTestId('send-now')).toBeNull();
    cleanup();
    const onSendNow = vi.fn();
    render(() => <SendButton running onSend={() => {}} onStop={() => {}} onSendNow={onSendNow} />);
    open();
    const now = screen.getByTestId('send-now');
    expect(now.textContent).toContain('Send now (stops run)');
    fireEvent.click(now);
    expect(onSendNow).toHaveBeenCalledTimes(1);
    // Choosing an action closes the menu.
    expect(screen.queryByTestId('send-now')).toBeNull();
  });

  it('Esc closes the menu and is consumed (it must not stop the run)', () => {
    let ancestorSawIt = false;
    const { container } = render(() => <SendButton running onSend={() => {}} onStop={() => {}} />);
    // A native ancestor listener (where the conversation's stop rule lives):
    // the popup consumed the Esc, so it must never arrive there.
    container.addEventListener('keydown', () => (ancestorSawIt = true));
    open();
    expect(screen.getByTestId('send-menu')).toBeTruthy();
    fireEvent.keyDown(screen.getByTestId('send-chevron'), { key: 'Escape' });
    expect(screen.queryByTestId('send-menu')).toBeNull();
    expect(ancestorSawIt).toBe(false);
  });
});
