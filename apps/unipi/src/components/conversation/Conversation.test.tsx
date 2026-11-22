// @vitest-environment jsdom
/**
 * The dock in the real conversation (WP-A9 acceptance, Z-46): with a dialog
 * waiting the composer still sends (a steer while the run goes), the pager
 * walks the dialogs in arrival order, and an answer leaves as
 * `extension_ui_response` on the session stream.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { createMockHost } from '../../dev/mock/host';
import { dialogDock } from '../../dev/mock/scenarios/dock';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';
import { resetHubs } from '../../lib/session/hub';
import { resetPanes } from '../../lib/panes';
import { resetUiRegistries } from '../../ui/registries';
import { Root } from '../../main';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

const HOST = 'pc-coffee';
const SESSION = 's001';

function boot(): ReturnType<typeof createMockHost> {
  setPlatformOverride('tauri-desktop');
  setDesktopOverride(true);
  Object.defineProperty(window, 'innerWidth', { configurable: true, value: 1440 });
  (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = { transformCallback: () => 0, invoke: () => Promise.resolve(0) };
  const world = createMockHost(dialogDock);
  setHostClientFactory(world.client);
  location.hash = `#/s/${HOST}/${SESSION}`;
  render(() => <Root />);
  return world;
}

afterEach(() => {
  cleanup();
  setDesktopOverride(undefined);
  setPlatformOverride(undefined);
  setHostClientFactory(undefined);
  resetHubs();
  resetPanes();
  resetLayout();
  resetUiRegistries();
  localStorage.clear();
  location.hash = '';
  delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
});

describe('dialog dock (acceptance)', () => {
  it('pages in arrival order, keeps the composer usable, and answers over the stream', async () => {
    const world = boot();
    await waitFor(() => expect(screen.getByTestId('dialog-panel')).toBeInTheDocument());
    expect(screen.getByTestId('dialog-count')).toHaveTextContent('1 of 2');
    expect(screen.getByText('Which file should I read first?')).toBeInTheDocument();

    fireEvent.click(screen.getByLabelText('Next dialog'));
    expect(screen.getByTestId('dialog-count')).toHaveTextContent('2 of 2');
    expect(screen.getByText('Overwrite README.md?')).toBeInTheDocument();
    fireEvent.click(screen.getByLabelText('Previous dialog'));

    // Typing during the waiting dialog still sends (the run is live → steer).
    const field = screen.getByTestId('tl-composer').querySelector('.chip-field') as HTMLDivElement;
    expect(field.getAttribute('contenteditable')).toBe('true');
    field.focus();
    field.textContent = 'skip the README for now';
    const range = document.createRange();
    range.selectNodeContents(field);
    range.collapse(false);
    const sel = window.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(range);
    fireEvent.input(field);
    fireEvent.keyDown(field, { key: 'Enter' });
    // WP-A11's matrix: Enter while running = the global default (queue ⇒ followUp).
    await waitFor(() => expect(world.log.some((e) => e.kind === 'cmd' && e.op === 'prompt' && (e.args as { streamingBehavior?: string }).streamingBehavior === 'followUp')).toBe(true));

    // The answer goes back as the pi response form; the dialog disappears.
    fireEvent.click(screen.getByText('src/math.ts — the failing test'));
    await waitFor(() =>
      expect(world.log.some((e) => e.kind === 'cmd' && e.op === 'extension_ui_response' && (e.args as { value?: unknown }).value === 'src/math.ts — the failing test')).toBe(true),
    );
    await waitFor(() => expect(screen.queryByText('Which file should I read first?')).toBeNull());
    // The second dialog is still there.
    expect(screen.getByTestId('dialog-panel')).toBeInTheDocument();
  });
});

describe('chat-vanish: the shown chat heals itself', () => {
  it('a handle closed under the shown chat (LRU eviction) is replaced within ~2 s: the composer sends again', async () => {
    const world = boot();
    const { sessions } = await import('../../lib/session/store');
    await waitFor(() => expect(sessions().peek(HOST, SESSION)?.state.conn).toBe('live'));
    sessions().peek(HOST, SESSION)!.close(); // what the eviction did to the chat on screen
    await new Promise((r) => setTimeout(r, 2_600));
    const field = screen.getByTestId('tl-composer').querySelector('.chip-field') as HTMLDivElement;
    field.focus();
    field.textContent = 'are you still there';
    const range = document.createRange();
    range.selectNodeContents(field);
    range.collapse(false);
    window.getSelection()!.removeAllRanges();
    window.getSelection()!.addRange(range);
    fireEvent.input(field);
    fireEvent.keyDown(field, { key: 'Enter' });
    // Before the fix the view kept the dead handle: the message sat "Waiting for connection…" forever.
    await waitFor(() => expect(world.log.some((e) => e.kind === 'cmd' && (e.args as { message?: string }).message === 'are you still there')).toBe(true), { timeout: 3_000 });
    expect(screen.queryByTestId('tl-open-failed')).toBeNull();
  }, 12_000);
});
