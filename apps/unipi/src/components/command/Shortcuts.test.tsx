/**
 * ShortcutsDialog (Ctrl+/) + ShortcutsSettings (Z-94): the table renders the
 * registry's chords (user rebinding applied), the resolved extension chords
 * with their rebindings noted, and the intercepted Shift+↓; rebinding a row
 * records the next key press and persists it (the app settings adapter here
 * is a fake — the real one is wired in AppShell).
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ShortcutsDialog, setShortcutsOpen } from '../shell/ShortcutsDialog';
import { ShortcutsSettings } from './ShortcutsSettings';
import { installShortcutsPersistence, rebindShortcut, resetShortcuts, setExtensionShortcuts } from '../../lib/shortcuts/registry';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')) }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));

let store: Record<string, string> = {};
beforeEach(() => {
  store = {};
  installShortcutsPersistence({ read: () => store, write: (next) => (store = next) });
});
afterEach(() => {
  cleanup();
  setShortcutsOpen(false);
  resetShortcuts();
});

describe('ShortcutsDialog (Ctrl+/)', () => {
  it('renders the registry table with the shell chords and screen keys', async () => {
    setShortcutsOpen(true);
    render(() => <ShortcutsDialog />);
    const rows = await screen.findAllByTestId('shortcut-row');
    const titles = rows.map((r) => r.textContent ?? '');
    expect(titles.some((t) => t.includes('Show / hide the sidebar'))).toBe(true);
    expect(titles.some((t) => t.includes('Command center'))).toBe(true);
    expect(titles.some((t) => t.includes('Find in chat'))).toBe(true);
    expect(screen.getByText('Board')).toBeInTheDocument(); // unipi screen keys kept (Z-94)
  });

  it('shows a user-rebound chord with the default noted', async () => {
    rebindShortcut('shell.sidebar', 'Mod+Shift+S');
    setShortcutsOpen(true);
    render(() => <ShortcutsDialog />);
    const row = screen.getAllByTestId('shortcut-row').find((r) => r.textContent?.includes('sidebar'))!;
    expect(/was (Ctrl\+B|⌘B)/.test(row.textContent ?? '')).toBe(true);
  });

  it('lists extension shortcuts: the Ctrl+B share with the sidebar + the intercepted Shift+↓ (G-7, P0-10)', async () => {
    setExtensionShortcuts([
      { id: 'unipi.subagents.foreground', chord: 'ctrl+b', title: 'Move foreground subagents to the background', extId: 'unipi.subagents' },
      { id: 'unipi.tray.open', chord: 'shift+down', title: 'Open the work tray', extId: 'unipi.core' },
    ]);
    setShortcutsOpen(true);
    render(() => <ShortcutsDialog />);
    await waitFor(() => {
      const rows = screen.getAllByTestId('shortcut-row').map((r) => r.textContent ?? '');
      const sub = rows.find((t) => t.replace(/\s+/g, '').includes('Moveforegroundsubagentstothebackground'));
      expect(sub && (sub.includes('Ctrl') || sub.includes('⌘')) && sub.includes('B') && !sub.includes('Alt')).toBe(true);
      expect(sub).toContain('while a subagent runs in the foreground; else the sidebar');
      expect(rows.some((t) => t.includes('Cycle the model'))).toBe(true);
      expect(rows.some((t) => t.includes('the app opens Session · Running instead'))).toBe(true);
    });
  });
});

describe('ShortcutsSettings (rebinding, Z-94)', () => {
  it('records the next chord press and persists it; reset restores the default', async () => {
    render(() => <ShortcutsSettings />);
    fireEvent.click(within(screen.getByTestId('sc-row-shell.sidebar')).getByText('Change'));
    expect(within(screen.getByTestId('sc-row-shell.sidebar')).getByText('Press keys…')).toBeInTheDocument();
    fireEvent.keyDown(window, { key: 's', code: 'KeyS', ctrlKey: true, shiftKey: true });
    await waitFor(() => expect(store['shell.sidebar']).toBe('Mod+Shift+S'));
    // rows() recreates its items on the chord change: re-query the live row
    const bound = () => screen.getByTestId('sc-row-shell.sidebar');
    await waitFor(() => expect(bound().textContent).toContain('Shift'));
    fireEvent.click(within(bound()).getByTitle(/Reset/));
    await waitFor(() => expect(store['shell.sidebar']).toBeUndefined());
    await waitFor(() => expect(bound().querySelector('[title^="Reset"]')).toBeNull());
  });

  it('Esc cancels recording without a change; Backspace resets to default', async () => {
    rebindShortcut('find.open', 'Mod+Shift+F2');
    render(() => <ShortcutsSettings />);
    const row = screen.getByTestId('sc-row-find.open');
    fireEvent.click(within(row).getByText('Change'));
    fireEvent.keyDown(window, { key: 'Escape' });
    await waitFor(() => expect(within(row).queryByText('Press keys…')).toBeNull());
    expect(store['find.open']).toBe('Mod+Shift+F2');
    fireEvent.click(within(row).getByText('Change'));
    fireEvent.keyDown(window, { key: 'Backspace' });
    await waitFor(() => expect(store['find.open']).toBeUndefined());
  });
});

function within(el: HTMLElement) {
  return {
    getByText: (text: string) => {
      const hit = [...el.querySelectorAll<HTMLElement>('button,span,kbd')].find((n) => n.textContent?.trim() === text);
      if (!hit) throw new Error(`no "${text}" in ${el.outerHTML.slice(0, 200)}`);
      return hit;
    },
    getByTitle: (title: string | RegExp) => {
      const hit = [...el.querySelectorAll<HTMLElement>('[title]')].find((n) => (typeof title === 'string' ? n.title === title : title.test(n.title)));
      if (!hit) throw new Error(`no title "${String(title)}"`);
      return hit;
    },
    queryByText: (text: string) => [...el.querySelectorAll<HTMLElement>('button,span,kbd')].find((n) => n.textContent?.trim() === text) ?? null,
  };
}
