// @vitest-environment jsdom
/** escapeStop (WP-A11, Z-55): Esc stops from any focus unless consumed. */
import { fireEvent } from '@solidjs/testing-library';
import { describe, expect, it, vi } from 'vitest';
import { escapeShouldStop, installEscapeStop } from './escapeStop';

const esc = { key: 'Escape', defaultPrevented: false, isComposing: false } as const;

describe('escapeShouldStop', () => {
  it('stops on a plain Esc while running', () => {
    expect(escapeShouldStop(esc, true)).toBe(true);
  });

  it('ignores non-Escape keys', () => {
    expect(escapeShouldStop({ ...esc, key: 'Enter' }, true)).toBe(false);
    expect(escapeShouldStop({ ...esc, key: 'q' }, true)).toBe(false);
  });

  it('ignores an Esc a dialog or popup consumed (defaultPrevented)', () => {
    expect(escapeShouldStop({ ...esc, defaultPrevented: true }, true)).toBe(false);
  });

  it('ignores Esc during IME composition', () => {
    expect(escapeShouldStop({ ...esc, isComposing: true }, true)).toBe(false);
  });

  it('never stops while no run is going', () => {
    expect(escapeShouldStop(esc, false)).toBe(false);
  });
});

describe('installEscapeStop', () => {
  it('stops from any focus inside the root and marks the event consumed', () => {
    const root = document.createElement('div');
    document.body.appendChild(root);
    const inner = document.createElement('input');
    root.appendChild(inner);
    const stop = vi.fn();
    const off = installEscapeStop({ root, running: () => true, stop });
    let documentDefaultPrevented = false;
    document.addEventListener('keydown', (e) => (documentDefaultPrevented = e.defaultPrevented));

    fireEvent.keyDown(inner, { key: 'Escape' });
    expect(stop).toHaveBeenCalledTimes(1);
    // The conversation consumed the Esc: document-level handlers see it consumed.
    expect(documentDefaultPrevented).toBe(true);

    off();
    fireEvent.keyDown(inner, { key: 'Escape' });
    expect(stop).toHaveBeenCalledTimes(1);
    expect(documentDefaultPrevented).toBe(false);
  });

  it('skips a consumed Esc (a dialog saw it first)', () => {
    const root = document.createElement('div');
    const dialog = document.createElement('div');
    root.appendChild(dialog);
    document.body.appendChild(root);
    const stop = vi.fn();
    installEscapeStop({ root, running: () => true, stop });
    // The dialog's own handler runs first in the bubble order.
    dialog.addEventListener('keydown', (e) => e.preventDefault());
    fireEvent.keyDown(dialog, { key: 'Escape' });
    expect(stop).not.toHaveBeenCalled();
  });
});
