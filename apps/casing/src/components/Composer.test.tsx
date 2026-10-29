// @vitest-environment jsdom
import { render, screen, fireEvent, cleanup } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { HistorySheet } from './Composer';
import { RecentsProvider } from '../lib/recentsContext';
import { backDepth } from '../lib/back';

afterEach(cleanup);

function mount() {
  const onClose = vi.fn();
  const view = render(() => (
    <RecentsProvider>
      <HistorySheet hostId="abc" onClose={onClose} onPick={vi.fn()} />
    </RecentsProvider>
  ));
  return { onClose, view };
}

describe('HistorySheet (UNI-128: a way back from "No sends yet")', () => {
  it('empty state has a Back and a Close button', () => {
    const { onClose } = mount();
    expect(screen.getByText('No sends yet.')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Back' }));
    fireEvent.click(screen.getByRole('button', { name: 'Close history' }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('Back (Escape outside Tauri) closes it while open; the handler pops on unmount', () => {
    const { onClose, view } = mount();
    expect(backDepth()).toBe(1);
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(onClose).toHaveBeenCalledTimes(1);
    view.unmount();
    expect(backDepth()).toBe(0);
  });
});
