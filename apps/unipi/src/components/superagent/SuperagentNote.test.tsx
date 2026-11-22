// @vitest-environment jsdom
/**
 * "Send as note" on the Superagent page: the Superagent chat and a lead chat opened from
 * it render the same `<Conversation superagent />`, so the send menu offers the note there
 * too and the command is `append_note` — never a prompt, steer or follow-up.
 */
import { cleanup, fireEvent, render, screen, waitFor, within } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { setHostClientFactory } from '../../lib/host/factory';
import { createMockHost } from '../../dev/mock/host';
import { superagentConcurrent } from '../../dev/mock/scenarios/superagent-concurrent';
import { Root } from '../../main';
import { resetSessions } from '../../lib/session/store';
import { resetHubs } from '../../lib/session/hub';
import { resetLayout, setDesktopOverride } from '../../lib/layout';
import { setPlatformOverride } from '../../lib/platform';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri')), transformCallback: () => 0 }));
afterEach(() => { cleanup(); vi.restoreAllMocks(); setHostClientFactory(undefined); resetSessions(); resetHubs(); resetLayout(); setDesktopOverride(undefined); setPlatformOverride(undefined); });

const NOTE_PLACEHOLDER = 'Note to yourself — the agent won’t see this';

function typeIntoComposer(text: string) {
  const field = document.querySelector('.chip-field') as HTMLDivElement;
  field.focus();
  field.textContent = text;
  const range = document.createRange();
  range.selectNodeContents(field);
  range.collapse(false);
  window.getSelection()!.removeAllRanges();
  window.getSelection()!.addRange(range);
  fireEvent.input(field);
  return field;
}

describe('Send as note on the Superagent page', () => {
  it('works in the Superagent chat and in a lead chat opened from it (desktop)', async () => {
    const world = createMockHost(superagentConcurrent);
    setHostClientFactory(world.client);
    setPlatformOverride('tauri-desktop'); setDesktopOverride(true);
    location.hash = '#/superagent?session=superagent-chat&host=pc-coffee';
    render(() => <Root />);
    await screen.findByTestId('superagent-page');
    const cmds = (op: string) => world.log.filter((r) => r.kind === 'cmd' && r.op === op);
    const saveNote = async (text: string) => {
      // the restricted chat keeps its send chevron, and the menu offers the note
      fireEvent.click(await screen.findByTestId('send-chevron'));
      expect(screen.getByTestId('send-as-note').textContent).toContain('Send as note');
      fireEvent.click(screen.getByTestId('send-as-note'));
      expect(screen.getByTestId('tl-composer').hasAttribute('data-note')).toBe(true);
      const field = typeIntoComposer(text);
      expect(field.getAttribute('data-placeholder')).toBe(NOTE_PLACEHOLDER);
      fireEvent.keyDown(field, { key: 'Enter' });
      await waitFor(() => expect(screen.getByTestId('tl').textContent).toContain(text));
    };

    // the Superagent chat: permission / plan / plug stay hidden, the note is saved as a note
    expect(screen.queryByTestId('permission-chip')).toBeNull();
    await saveNote('note in the superagent chat');
    expect(cmds('append_note')).toHaveLength(1);
    expect((cmds('append_note')[0]!.args as { text: string }).text).toBe('note in the superagent chat');
    for (const op of ['prompt', 'steer', 'follow_up']) expect(cmds(op)).toHaveLength(0);

    // a lead chat opened from the Superagent history: same conversation, same menu
    fireEvent.click(await screen.findByLabelText('Superagent history'));
    fireEvent.click(within(screen.getByLabelText('Superagent history list')).getByRole('button', { name: /Lead · Oino/ }));
    await waitFor(() => expect(location.hash).toContain('session=lead-oino'));
    await waitFor(() => expect(screen.getByTestId('tl-composer').hasAttribute('data-note')).toBe(false));
    await saveNote('note in a lead chat');
    expect(cmds('append_note')).toHaveLength(2);
    for (const op of ['prompt', 'steer', 'follow_up']) expect(cmds(op)).toHaveLength(0);
  });

  it('works in the Superagent chat on the phone: the menu item is there and Save note stays reachable', async () => {
    const world = createMockHost(superagentConcurrent);
    setHostClientFactory(world.client);
    setPlatformOverride('tauri-desktop'); setDesktopOverride(false);
    location.hash = '#/superagent?session=superagent-chat&host=pc-coffee';
    render(() => <Root />);
    await screen.findByTestId('superagent-page');
    fireEvent.click(await screen.findByTestId('send-chevron'));
    fireEvent.click(screen.getByTestId('send-as-note'));
    expect(screen.getByTestId('composer-note-chip')).toBeTruthy();
    // the chat is mid-run here: an empty composer shows Stop, a typed note shows Save note
    const field = screen.getByTestId('phone-field') as HTMLTextAreaElement;
    field.value = 'phone note';
    fireEvent.input(field);
    const save = screen.getByLabelText('Save note');
    expect(save).toBe(screen.getByTestId('send-send'));
    fireEvent.click(save);
    await waitFor(() => expect(world.log.filter((r) => r.kind === 'cmd' && r.op === 'append_note')).toHaveLength(1));
    await waitFor(() => expect(screen.getByTestId('tl').textContent).toContain('phone note'));
    await waitFor(() => expect(field.value).toBe(''));
  });
});
