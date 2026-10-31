// @vitest-environment jsdom
/**
 * The app-wide toast stack UI (WP-A19, Z-85): rendering, the one action, the
 * ✕, folding ×N, the error's alert role, and hover pausing the timer.
 */
import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { Toasts } from './Toasts';
import { createToastStack, setToasts, type ToastStack } from '../lib/toasts';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

let stack: ToastStack;
const timers: Array<{ ms: number; cb: () => void }> = [];

beforeEach(() => {
  timers.length = 0;
  stack = createToastStack(
    {
      set: (ms, cb) => {
        const id = { ms, cb };
        timers.push(id);
        return id;
      },
      clear: (h) => {
        const i = timers.indexOf(h as { ms: number; cb: () => void });
        if (i >= 0) timers.splice(i, 1);
      },
    },
    () => 0,
  );
  setToasts(stack);
});
afterEach(() => {
  cleanup();
  setToasts(undefined);
});

describe('Toasts', () => {
  it('renders the stack: icon, message, ×N fold, one action, dismiss ✕', () => {
    stack.push('Committed abc1234');
    stack.push('Committed abc1234');
    stack.push('Task archived', 'info', { label: 'Undo', run: () => {} });
    render(() => <Toasts />);
    expect(screen.getByText('Committed abc1234')).toBeTruthy();
    expect(screen.getByText('×2')).toBeTruthy();
    const action = screen.getByRole('button', { name: /undo/i });
    expect(action).toBeTruthy();
    expect(screen.getAllByRole('button', { name: 'Dismiss' })).toHaveLength(2);
  });

  it('an action click dismisses the toast and runs the action', () => {
    const run = vi.fn();
    stack.push('Task archived', 'info', { label: 'Undo', run });
    render(() => <Toasts />);
    fireEvent.click(screen.getByRole('button', { name: /undo/i }));
    expect(run).toHaveBeenCalledTimes(1);
    expect(stack.list()[0]!.leaving).toBe(true);
  });

  it('errors announce as alerts, the rest as status', () => {
    stack.push('ok');
    stack.push('bad', 'error');
    render(() => <Toasts />);
    expect(screen.getByRole('status').textContent).toContain('ok');
    expect(screen.getByRole('alert').textContent).toContain('bad');
  });

  it('hovering pauses the toast timer, leaving resumes it (Z-85)', () => {
    const pause = vi.spyOn(stack, 'pause');
    const resume = vi.spyOn(stack, 'resume');
    stack.push('Copied');
    const { container } = render(() => <Toasts />);
    const toast = container.querySelector('.app-toast')!;
    fireEvent.pointerEnter(toast);
    expect(pause).toHaveBeenCalledWith(stack.list()[0]!.id);
    fireEvent.pointerLeave(toast);
    expect(resume).toHaveBeenCalledWith(stack.list()[0]!.id);
  });

  it('clicking anywhere on a toast dismisses it', () => {
    stack.push('Plan mode on');
    const { container } = render(() => <Toasts />);
    const toast = container.querySelector('.app-toast')!;
    fireEvent.click(toast);
    expect(stack.list()[0]!.leaving).toBe(true);
  });
});
