import { cleanup, fireEvent, render, screen } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { CommandPalette } from './CommandPalette';
import { registerCommands, setPaletteOpen, paletteOpen } from '../../lib/shell';

afterEach(() => {
  setPaletteOpen(false);
  cleanup();
});

describe('<CommandPalette>', () => {
  it('lists commands, filters, runs with Enter, closes', async () => {
    const run = vi.fn();
    const off = registerCommands([
      { id: 'p.one', title: 'Open the pod bay doors', group: 'Ship', run },
      { id: 'p.two', title: 'Go to Board', group: 'Go to', keys: 'Ctrl 3', run: () => {} },
    ]);
    render(() => <CommandPalette />);
    expect(screen.queryByTestId('command-palette')).toBeNull();
    setPaletteOpen(true);
    const input = await screen.findByLabelText('Command');
    expect(screen.getByText('Open the pod bay doors')).toBeInTheDocument();
    fireEvent.input(input, { target: { value: 'pod' } });
    expect(screen.queryByText('Go to Board')).toBeNull();
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(paletteOpen()).toBe(false);
    await Promise.resolve();
    expect(run).toHaveBeenCalledOnce();
    off();
  });

  it('arrow keys move the selection; empty result says so; Escape closes', async () => {
    const off = registerCommands([
      { id: 'q.a', title: 'Alpha', group: 'G', run: () => {} },
      { id: 'q.b', title: 'Beta', group: 'G', run: () => {} },
    ]);
    render(() => <CommandPalette />);
    setPaletteOpen(true);
    const input = await screen.findByLabelText('Command');
    fireEvent.input(input, { target: { value: 'q' } }); // no title contains q → none
    fireEvent.input(input, { target: { value: '' } });
    const options = () => screen.getAllByRole('option');
    const selected = () => options().findIndex((o) => o.getAttribute('aria-selected') === 'true');
    const start = selected();
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    expect(selected()).toBe(start + 1);
    fireEvent.keyDown(input, { key: 'ArrowUp' });
    expect(selected()).toBe(start);
    fireEvent.input(input, { target: { value: 'zzzz' } });
    expect(screen.getByText('No matching commands')).toBeInTheDocument();
    fireEvent.keyDown(window, { key: 'Escape' });
    expect(paletteOpen()).toBe(false);
    off();
  });
});
