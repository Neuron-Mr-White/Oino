// @vitest-environment jsdom
/**
 * The new-chat draft's input is the chat's own multi-line composer, not a one-line
 * field: Enter sends, Shift+Enter is a newline (never a send), and pasted or dropped
 * images become pending image chips through the same intake the chat uses.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { setDesktopOverride } from '../../lib/layout';
import { resetComposerDrafts } from '../../lib/composer/drafts';
import { resetAttachments } from '../../lib/composer/attachments';
import { setBlobTransport } from '../../lib/blob';
import { resetHostCache } from './hostCache';

const start = vi.hoisted(() => vi.fn());
vi.mock('./startSession', () => ({ startSession: start, LINGER_MS: 0 }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));
vi.mock('../../lib/imageShrink', () => ({
  MAX_SIDE: 1568,
  QUALITY: 0.8,
  INLINE_MAX_BYTES: 700 * 1024,
  shrinkImage: async (file: Blob) => ({ blob: file, mime: 'image/png', width: 100, height: 80 }),
  blobToBase64: async () => 'QUJD',
}));

import { DraftView } from './DraftView';

const boot = (commandReply?: Promise<unknown>) => {
  const scenario: MockScenario = {
    name: 't',
    hosts: [
      {
        id: 'this-pc',
        name: 'workstation',
        transport: 'local',
        conn: { state: 'connected' },
        cards: [],
        commands: ['swarm', 'graph', 'goal', 'ralph'].map((name) => ({ name, source: 'extension', description: `Start ${name}` })),
        fs: { '/home/you': { kind: 'dir', children: {} } },
        settings: {
          pi: { defaultProvider: 'oino', defaultModel: 'deepseek-flash' },
          extensions: [{ id: 'unipi.workflow', kind: 'extension', enabled: true, loaded: true }],
          providers: [{ id: 'oino', configured: true }],
        },
      },
    ],
  };
  const world = createMockHost(scenario);
  const wrapped = new Set<string>();
  setHostClientFactory((id) => {
    const client = world.client(id);
    if (!commandReply || wrapped.has(id)) return client;
    wrapped.add(id);
    const request = client.request.bind(client);
    client.request = <T,>(op: string, args?: Record<string, unknown>): Promise<T> => {
      if (op === 'host_cmd' && (args?.cmd as { kind?: string })?.kind === 'get_commands') return commandReply as Promise<T>;
      return request<T>(op, args);
    };
    return client;
  });
  location.hash = '#/new/this-pc';
  render(() => (
    <SettingsProvider>
      <HostsProvider>
        <HashRouter>
          <Route path="/new/:host" component={() => <DraftView hostId="this-pc" />} />
          <Route path="/s/:host/:id" component={() => <div data-testid="chat-route" />} />
        </HashRouter>
      </HostsProvider>
    </SettingsProvider>
  ));
};

const field = () => document.querySelector('.chip-field') as HTMLDivElement;
const chips = () => [...document.querySelectorAll('.chip-field .ichip[data-kind=image]')] as HTMLElement[];
const typeInto = (text: string) => {
  const el = field();
  el.focus();
  el.textContent = text;
  const range = document.createRange();
  range.selectNodeContents(el);
  range.collapse(false);
  window.getSelection()!.removeAllRanges();
  window.getSelection()!.addRange(range);
  fireEvent.input(el);
};
const clip = (type: 'paste' | 'drop', files: File[]) => {
  const data = { files, items: files.map((file) => ({ kind: 'file', type: file.type, getAsFile: () => file })), types: ['Files'], getData: () => '' };
  const event = new Event(type, { bubbles: true, cancelable: true });
  Object.defineProperty(event, type === 'paste' ? 'clipboardData' : 'dataTransfer', { value: data });
  field().dispatchEvent(event);
  return event;
};

beforeEach(() => {
  localStorage.clear();
  resetHostCache();
  resetComposerDrafts();
  resetAttachments();
  setDesktopOverride(true);
  start.mockReset();
  start.mockResolvedValue({ ok: true, sessionId: 's-new' });
  setBlobTransport({
    async putBytes(_host, bytes) {
      return { sha256: 'a'.repeat(64), size: bytes.length, path: '/blob', deduped: false };
    },
    async putFile() {
      throw new Error('unused');
    },
    async getFile() {
      throw new Error('unused');
    },
    async getBytes() {
      throw new Error('unused');
    },
  });
});
afterEach(() => {
  cleanup();
  setBlobTransport(undefined);
  setHostClientFactory(undefined);
  resetHubs();
});

describe('new-chat input is the multi-line composer', () => {
  it('Shift+Enter is a newline (no send); Enter sends', async () => {
    boot();
    await waitFor(() => expect(field()).toBeTruthy());
    expect(field().getAttribute('contenteditable')).not.toBe('false');
    expect(field().getAttribute('aria-multiline')).toBe('true');
    typeInto('first line');
    const shifted = fireEvent.keyDown(field(), { key: 'Enter', shiftKey: true });
    await new Promise((r) => setTimeout(r, 20));
    expect(start).not.toHaveBeenCalled();
    // Not swallowed as a send: the browser may insert the line break.
    expect(shifted).toBe(true);
    fireEvent.keyDown(field(), { key: 'Enter' });
    await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
    expect(start.mock.calls[0]![1].text).toBe('first line');
  });

  for (const method of ['paste', 'drop'] as const) {
    it(`${method}ing an image makes a pending image chip, sent with the first prompt`, async () => {
      boot();
      await waitFor(() => expect(field()).toBeTruthy());
      typeInto('what is this?');
      const event = clip(method, [new File([new Uint8Array(2048)], 'shot.png', { type: 'image/png' })]);
      expect(event.defaultPrevented).toBe(true);
      await waitFor(() => expect(chips()).toHaveLength(1));
      await waitFor(() => expect(chips()[0]!.getAttribute('data-status')).toBe('done'));
      await waitFor(() => expect((screen.getByTestId('send-send') as HTMLButtonElement).disabled).toBe(false));
      fireEvent.click(screen.getByTestId('send-send'));
      await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
      expect(start.mock.calls[0]![1].images).toHaveLength(1);
    });
  }
});

describe('N4: first-chat extension commands', () => {
  it('lists host extension commands in the draft slash menu and starts the first swarm', async () => {
    boot();
    await waitFor(() => expect(field()).toBeTruthy());
    typeInto('/sw');
    await waitFor(() => expect(document.querySelector('[data-testid=slash-menu]')?.textContent).toContain('/swarm'));
    typeInto('/swarm first task');
    fireEvent.keyDown(field(), { key: 'Escape' });
    fireEvent.keyDown(field(), { key: 'Enter' });
    await waitFor(() => expect(start).toHaveBeenCalledWith('this-pc', expect.objectContaining({ text: '/swarm first task' })));
    expect(document.body.textContent).not.toContain('Unknown command');
  });
  it('keeps the text and holds Enter until command discovery completes', async () => {
    let resolve!: (value: unknown) => void;
    boot(new Promise((done) => { resolve = done; }));
    await waitFor(() => expect(field()).toBeTruthy());
    typeInto('/swarm');
    expect(screen.getByTestId('draft-commands-status').textContent).toContain('Loading commands');
    typeInto('/swarm first task');
    fireEvent.keyDown(field(), { key: 'Escape' });
    fireEvent.keyDown(field(), { key: 'Enter' });
    expect(start).not.toHaveBeenCalled();
    expect(field().textContent).toContain('/swarm first task');
    expect(document.body.textContent).not.toContain('Unknown command');
    resolve({ commands: [{ name: 'swarm', source: 'extension' }] });
    await waitFor(() => expect(screen.getByTestId('send-send').hasAttribute('disabled')).toBe(false));
    fireEvent.keyDown(field(), { key: 'Enter' });
    await waitFor(() => expect(start).toHaveBeenCalled());
  });
});
