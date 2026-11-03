// @vitest-environment jsdom
/**
 * Draft view (Z-19, WP-A5): `/new/:host` renders the chat's own composer
 * (same toolbar) with a workspace row inside the card; the first prompt
 * starts the session in the chosen folder (`startSession`: `session_new`
 * stream + the prompt) and routes to `/s/:host/:id`. The draft's own stand-in
 * session keeps the permission / Plan / thinking choices for that start.
 */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { createRoot } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { resetHubs } from '../../lib/session/hub';
import { setDesktopOverride } from '../../lib/layout';
import { composerDraft, resetComposerDrafts } from '../../lib/composer/drafts';
import { queueFirstPrompt, takeFirstPrompt } from '../../lib/session/firstPrompt';
import type { SessionCard } from '../../lib/host/client';
import { createDraftSession, draftKey } from './draftSession';
import { homeOf, parentOf } from './WorkspacePicker';
import { rememberCommands, rememberModels } from './hostCache';

const start = vi.hoisted(() => vi.fn());
vi.mock('./startSession', () => ({ startSession: start, LINGER_MS: 0 }));
vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));

import { DraftView } from './DraftView';

const card = (id: string, cwd: string): SessionCard => ({
  session_id: id,
  name: `Session ${id}`,
  cwd,
  run_state: 'idle',
  updated_at: Date.now() - 90_000,
  message_count: 2,
});

const boot = (providers: Array<{ id: string; configured?: boolean }> = [{ id: 'oino', configured: true }], extraExtensions: Array<{ id: string; kind: string; enabled: boolean; loaded: boolean }> = []) => {
  const scenario: MockScenario = {
    name: 't',
    hosts: [
      {
        id: 'this-pc',
        name: 'workstation',
        transport: 'local',
        conn: { state: 'connected' },
        cards: [card('a', '/home/you/p/oino')],
        fs: { '/home/you': { kind: 'dir', children: { p: { kind: 'dir', children: {} } } }, '/srv': { kind: 'dir', children: {} } },
        settings: {
          pi: { defaultProvider: 'oino', defaultModel: 'deepseek-flash' },
          extensions: [{ id: 'unipi.workflow', kind: 'extension', enabled: true, loaded: true }, ...extraExtensions],
          providers,
        },
      },
    ],
  };
  const world = createMockHost(scenario);
  setHostClientFactory(world.client);
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
  return world;
};

const field = () => document.querySelector('.chip-field') as HTMLDivElement;
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

beforeEach(() => {
  localStorage.clear();
  resetComposerDrafts();
  setDesktopOverride(true);
  start.mockReset();
});
afterEach(() => {
  cleanup();
  setHostClientFactory(undefined);
  resetHubs();
});

describe('DraftView (Z-19)', () => {
  it('renders the chat composer with its toolbar and a workspace row', async () => {
    boot();
    await waitFor(() => expect(screen.getByTestId('draft-view')).toBeTruthy());
    expect(screen.getByTestId('tl-composer')).toBeTruthy();
    expect(screen.getByTestId('composer-toolbar')).toBeTruthy();
    expect(screen.getByTestId('composer-plus')).toBeTruthy();
    expect(screen.getByTestId('send-send')).toBeTruthy();
    expect(screen.getByTestId('workspace-chip').textContent).toContain('No workspace');
    // The host's default model; the draft has no context to measure yet.
    await waitFor(() => expect(screen.getByTestId('model-chip-label').textContent).toContain('DeepSeek Flash'));
    expect(screen.queryByTestId('context-ring')).toBeNull();
    // The workflow extension runs on this PC → its controls are there.
    await waitFor(() => expect(screen.getByTestId('permission-chip')).toBeTruthy());
    expect(screen.getByTestId('plan-toggle')).toBeTruthy();
    // No old pill rows and no bare textarea.
    expect(document.querySelector('textarea')).toBeNull();
    expect(screen.queryByTestId('draft-folders')).toBeNull();
  });

  it('D-01: no working provider → "Choose a model" chip and a Connect a provider card', async () => {
    boot([{ id: 'oino', configured: false }]);
    await waitFor(() => expect(screen.getByTestId('connect-provider-card')).toBeTruthy());
    expect(screen.getByTestId('model-chip-label').textContent).toBe('Choose a model');
  });

  it('D-01: a working provider shows the model and no card', async () => {
    boot();
    await waitFor(() => expect(screen.getByTestId('model-chip-label').textContent).toContain('DeepSeek Flash'));
    expect(screen.queryByTestId('connect-provider-card')).toBeNull();
  });

  it('the workspace menu lists recent folders (no system roots); a pick shows in the chip', async () => {
    boot();
    await waitFor(() => expect(screen.getByTestId('workspace-chip')).toBeTruthy());
    await new Promise((r) => setTimeout(r, 30));
    fireEvent.click(screen.getByTestId('workspace-chip'));
    const menu = await waitFor(() => screen.getByTestId('workspace-menu'));
    await waitFor(() => expect(menu.textContent).toContain('~/p/oino'));
    expect(menu.textContent).not.toMatch(/On this PC|\/srv/i);
    expect(menu.textContent).toContain('Open folder…');
    fireEvent.click([...menu.querySelectorAll('.ws-row')].find((r) => r.textContent?.includes('~/p/oino'))!);
    await waitFor(() => expect(screen.getByTestId('workspace-chip').textContent).toContain('oino'));
  });

  it('the folder browser starts with a Recent group (owner 2026-11-03), then the folders', async () => {
    boot();
    await waitFor(() => expect(screen.getByTestId('workspace-chip')).toBeTruthy());
    await new Promise((r) => setTimeout(r, 30));
    fireEvent.click(screen.getByTestId('workspace-chip'));
    const menu = await waitFor(() => screen.getByTestId('workspace-menu'));
    await waitFor(() => expect(menu.textContent).toContain('~/p/oino'));
    fireEvent.click(screen.getByTestId('workspace-open'));
    const browser = await screen.findByTestId('workspace-browser');
    const recent = await waitFor(() => screen.getByTestId('workspace-browse-recent'));
    expect(browser.firstElementChild).toBe(recent);
    expect(recent.textContent).toContain('Recent');
    expect(recent.textContent).toContain('~/p/oino');
    fireEvent.click([...recent.querySelectorAll('.ws-row')].find((r) => r.textContent?.includes('~/p/oino'))!);
    await waitFor(() => expect(screen.getByTestId('workspace-chip').textContent).toContain('oino'));
  });

  it('the first prompt starts the session in the workspace and routes to it', async () => {
    start.mockResolvedValue({ ok: true, sessionId: 's-new-1' });
    boot();
    await waitFor(() => expect(field()).toBeTruthy());
    typeInto('Triage the failing tests');
    fireEvent.keyDown(field(), { key: 'Enter' });
    await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
    const [host, req] = start.mock.calls[0]!;
    expect(host).toBe('this-pc');
    expect(req.text).toBe('Triage the failing tests');
    // No workspace → no folder: the host runs the chat in its private scratch folder, never home.
    expect(req.cwd).toBeUndefined();
    await waitFor(() => expect(screen.getByTestId('chat-route')).toBeTruthy());
  });

  it('a failed start keeps the message and says why', async () => {
    start.mockResolvedValue({ ok: false, error: { code: 'E_FAILED', message: 'The host is busy', retryable: true } });
    boot();
    await waitFor(() => expect(field()).toBeTruthy());
    typeInto('Hello there');
    fireEvent.keyDown(field(), { key: 'Enter' });
    await waitFor(() => expect(screen.getByTestId('draft-error').textContent).toContain('The host is busy'));
    expect(composerDraft(draftKey('this-pc')).text).toBe('Hello there');
  });

  it('F-01: with no provider Enter/Send never sends and never clears the typed text', async () => {
    boot([{ id: 'oino', configured: false }]);
    await waitFor(() => expect(screen.getByTestId('connect-provider-card')).toBeTruthy());
    typeInto('hello no provider');
    const send = () => screen.getByTestId('send-send') as HTMLButtonElement;
    await waitFor(() => expect(send().disabled).toBe(true));
    fireEvent.keyDown(field(), { key: 'Enter' });
    fireEvent.click(send());
    await new Promise((r) => setTimeout(r, 20));
    expect(start).not.toHaveBeenCalled();
    expect(composerDraft(draftKey('this-pc')).text).toBe('hello no provider');
    expect(screen.getByTestId('connect-provider-card')).toBeTruthy();
  });

  it('an empty draft cannot send', async () => {
    boot();
    await waitFor(() => expect(screen.getByTestId('send-send')).toBeTruthy());
    expect((screen.getByTestId('send-send') as HTMLButtonElement).disabled).toBe(true);
  });
});

describe('draft session (stand-in handle)', () => {
  it('keeps permission / plan / thinking choices instead of sending them', async () => {
    const world = createMockHost({ name: 't', hosts: [{ id: 'h', name: 'h', conn: { state: 'connected' } }] });
    setHostClientFactory(world.client);
    const submitted: string[] = [];
    const s = createRoot(() => createDraftSession('h', (text) => submitted.push(text)));
    s.send('/permission full', { mode: 'prompt' });
    s.send('/plan on', { mode: 'prompt' });
    await s.cmd({ type: 'set_thinking_level', level: 'high' });
    expect(s.choices()).toEqual({ permission: 'full', plan: true, thinking: 'high' });
    expect((s.state.topics['unipi:workflow:status']?.data as { planMode: boolean }).planMode).toBe(true);
    expect(s.state.thinkingLevel).toBe('high');
    expect(submitted).toEqual([]);
    s.send('Go', { mode: 'prompt' });
    expect(submitted).toEqual(['Go']);
  });
});

describe('the draft model chip opens the ONE model dialog', () => {
  it('same dialog as the chat (Now, search, Cycle set chip, list with ⋯, scope line); a pick sets the draft; Use at startup writes the host default', async () => {
    rememberModels('this-pc', [
      { provider: 'oino', id: 'deepseek-flash', name: 'DeepSeek Flash' },
      { provider: 'oino', id: 'glm-4.7', name: 'GLM 4.7' },
    ]);
    const world = boot();
    await waitFor(() => expect(screen.getByTestId('model-chip-label').textContent).toContain('DeepSeek Flash'));
    fireEvent.click(screen.getByTestId('model-chip'));
    const menu = await screen.findByTestId('draft-model-menu');
    expect(menu.getAttribute('role')).toBe('dialog');
    expect(menu.getAttribute('aria-label')).toBe('Model');
    expect(screen.getByTestId('model-picker-overlay').contains(menu)).toBe(true);
    expect(menu.querySelector('[data-testid="model-search"]')).toBeTruthy();
    await waitFor(() => expect(screen.getByTestId('model-picker-list').textContent).toContain('GLM 4.7'));
    // "Now" is the model the new chat will start with.
    expect(screen.getByTestId('model-picker-now').textContent).toContain('DeepSeek Flash');
    expect(screen.getByTestId('model-picker-scope').textContent).toContain('the chat about to start');
    expect(screen.getByTestId('model-filter-cycle').textContent).toContain('Cycle set');
    expect(screen.queryByTestId('model-picker-cycle')).toBeNull();
    expect(screen.getAllByTestId('model-row-more').length).toBeGreaterThan(0);
    // No Fusion tab: unipi.fusion is not enabled on this PC.
    expect(screen.queryByTestId('model-tab-fusion')).toBeNull();
    // The host default is the running pick: the same enabled "Use at startup" as a chat's, checked.
    await waitFor(() => expect(screen.getByTestId('model-use-startup').getAttribute('aria-pressed')).toBe('true'));
    expect(screen.getByTestId('model-use-startup').textContent).toBe('Use at startup');
    expect((screen.getByTestId('model-use-startup') as HTMLButtonElement).disabled).toBe(false);
    // The ⋯ menu: no Fusion here (unipi.fusion off), cycle set + startup.
    fireEvent.click(screen.getAllByTestId('model-row-more')[0]!);
    expect(await screen.findByTestId('model-menu-cycle')).toBeTruthy();
    expect(screen.getByTestId('model-menu-startup')).toBeTruthy();
    expect(screen.queryByTestId('model-menu-lead')).toBeNull();
    fireEvent.keyDown(screen.getByTestId('model-row-menu'), { key: 'Escape' });
    fireEvent.click(screen.getByTestId('model-picker-list').querySelector('[data-key="oino/glm-4.7"]')!);
    await waitFor(() => expect(screen.queryByTestId('draft-model-menu')).toBeNull());
    await waitFor(() => expect(screen.getByTestId('model-chip-label').textContent).toContain('GLM 4.7'));
    fireEvent.click(screen.getByTestId('model-chip'));
    await waitFor(() => expect(screen.getByTestId('model-use-startup').textContent).toContain('Use at startup'));
    expect(screen.getByTestId('model-use-startup').getAttribute('aria-pressed')).toBe('false');
    fireEvent.click(screen.getByTestId('model-use-startup'));
    await waitFor(() => expect(screen.getByTestId('model-use-startup').getAttribute('aria-pressed')).toBe('true'));
    const sets = world.log
      .filter((l) => l.op === 'host_cmd')
      .map((l) => (l.args as { cmd?: { kind?: string; key?: string; value?: unknown } }).cmd)
      .filter((c) => c?.kind === 'set_setting');
    expect(sets).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ key: 'defaultProvider', value: 'oino' }),
        expect.objectContaining({ key: 'defaultModel', value: 'glm-4.7' }),
      ]),
    );
  });
});

describe('the draft model dialog has a Fusion tab when unipi.fusion runs on this PC', () => {
  it('picks a lead + sidekick pair for the new chat; the chip and Now show it; the start carries it; a Single pick turns it off', async () => {
    start.mockResolvedValue({ ok: true, sessionId: 's-new-2' });
    rememberModels('this-pc', [
      { provider: 'oino', id: 'deepseek-flash', name: 'DeepSeek Flash' },
      { provider: 'oino', id: 'glm-4.7', name: 'GLM 4.7' },
      { provider: 'oino', id: 'kimi-k2', name: 'Kimi K2' },
    ]);
    boot(undefined, [{ id: 'unipi.fusion', kind: 'extension', enabled: true, loaded: true }]);
    await waitFor(() => expect(screen.getByTestId('model-chip-label').textContent).toContain('DeepSeek Flash'));
    fireEvent.click(screen.getByTestId('model-chip'));
    await screen.findByTestId('draft-model-menu');
    fireEvent.click(await screen.findByTestId('model-tab-fusion'));
    const lead = await screen.findByTestId('fusion-col-lead');
    const sidekick = screen.getByTestId('fusion-col-sidekick');
    expect(screen.getByTestId('model-picker-scope').textContent).toContain('right after it starts');
    const row = (col: HTMLElement, name: string) => [...col.querySelectorAll<HTMLButtonElement>('.mp-row')].find((r) => r.textContent?.includes(name))!;
    fireEvent.click(row(lead, 'GLM 4.7'));
    fireEvent.click(row(sidekick, 'Kimi K2'));
    fireEvent.click(screen.getByRole('group', { name: 'Lead effort' }).querySelector('button:last-child')!);
    expect(screen.getByTestId('draft-fusion-use').textContent).toBe('Use this pair');
    fireEvent.click(screen.getByTestId('draft-fusion-use'));
    await waitFor(() => expect(screen.queryByTestId('draft-model-menu')).toBeNull());
    await waitFor(() => expect(screen.getByTestId('model-chip-label').textContent).toBe('Fusion · GLM 4.7 ◆ Kimi K2'));
    // Reopened: the Fusion tab, Now = the pair, the pair is the current one.
    fireEvent.click(screen.getByTestId('model-chip'));
    await screen.findByTestId('fusion-col-lead');
    expect(screen.getByTestId('model-picker-now').textContent).toContain('Fusion · GLM 4.7 ◆ Kimi K2');
    expect(screen.getByTestId('draft-fusion-use').textContent).toBe('Current pair');
    fireEvent.click(screen.getByTestId('draft-model-menu').querySelector('[aria-label="Close"]') ?? screen.getByTestId('model-chip'));
    await waitFor(() => expect(screen.queryByTestId('draft-model-menu')).toBeNull());
    // The first prompt starts the chat with the pair (applied right after it starts).
    typeInto('Pair up');
    fireEvent.keyDown(field(), { key: 'Enter' });
    await waitFor(() => expect(start).toHaveBeenCalledTimes(1));
    const [, req] = start.mock.calls[0]!;
    expect(req.choices.fusion).toEqual({ lead: 'oino/glm-4.7', sidekick: 'oino/kimi-k2', leadEffort: 'high', sidekickEffort: 'medium' });
  });

  it('the row ⋯ menu matches a chat\'s: Set as Fusion lead / sidekick set the draft\'s pair (owner 2026-11-03)', async () => {
    rememberModels('this-pc', [
      { provider: 'oino', id: 'deepseek-flash', name: 'DeepSeek Flash' },
      { provider: 'oino', id: 'glm-4.7', name: 'GLM 4.7' },
      { provider: 'oino', id: 'kimi-k2', name: 'Kimi K2' },
    ]);
    boot(undefined, [{ id: 'unipi.fusion', kind: 'extension', enabled: true, loaded: true }]);
    await waitFor(() => expect(screen.getByTestId('model-chip-label').textContent).toContain('DeepSeek Flash'));
    fireEvent.click(screen.getByTestId('model-chip'));
    await screen.findByTestId('draft-model-menu');
    await waitFor(() => expect(screen.getByTestId('model-picker-list').querySelector('[data-key="oino/kimi-k2"]')).toBeTruthy());
    const more = (key: string) => screen.getByTestId('model-picker-list').querySelector<HTMLElement>(`[data-key="${key}"]`)!.closest('.mp-row-wrap, li, div')!.querySelector<HTMLElement>('[data-testid="model-row-more"]')!;
    fireEvent.click(more('oino/kimi-k2'));
    const menu = await screen.findByTestId('model-row-menu');
    expect([...menu.querySelectorAll('[role^="menuitem"]')].map((b) => b.textContent)).toEqual(['Set as Fusion lead', 'Set as Fusion sidekick', 'Add to cycle set', 'Use at startup']);
    fireEvent.click(screen.getByTestId('model-menu-sidekick'));
    await waitFor(() => expect(screen.getByTestId('model-chip-label').textContent).toBe('Fusion · DeepSeek Flash ◆ Kimi K2'));
  });

  it('phone: the full-screen model sheet has the same Fusion tab and chip label', async () => {
    setDesktopOverride(false);
    rememberModels('this-pc', [
      { provider: 'oino', id: 'deepseek-flash', name: 'DeepSeek Flash' },
      { provider: 'oino', id: 'glm-4.7', name: 'GLM 4.7' },
    ]);
    boot(undefined, [{ id: 'unipi.fusion', kind: 'extension', enabled: true, loaded: true }]);
    await waitFor(() => expect(screen.getByTestId('model-chip-label').textContent).toContain('DeepSeek Flash'));
    fireEvent.click(screen.getByTestId('model-chip'));
    await screen.findByTestId('draft-model-menu');
    fireEvent.click(await screen.findByTestId('model-tab-fusion'));
    const sidekick = await screen.findByTestId('fusion-col-sidekick');
    fireEvent.click([...sidekick.querySelectorAll<HTMLButtonElement>('.mp-row')].find((r) => r.textContent?.includes('GLM 4.7'))!);
    fireEvent.click(screen.getByTestId('draft-fusion-use'));
    await waitFor(() => expect(screen.getByTestId('model-chip-label').textContent).toBe('Fusion · DeepSeek Flash ◆ GLM 4.7'));
  });

  it('a Single pick on the draft turns its pair off', async () => {
    const world = createMockHost({ name: 't', hosts: [{ id: 'h', name: 'h', conn: { state: 'connected' } }] });
    setHostClientFactory(world.client);
    const s = createRoot(() => createDraftSession('h', () => {}));
    s.pickFusion({ lead: 'oino/a', sidekick: 'oino/b', leadEffort: 'medium', sidekickEffort: 'low' }, { provider: 'oino', modelId: 'a', name: 'A' });
    expect(s.choices().fusion?.sidekick).toBe('oino/b');
    expect(s.state.model?.id).toBe('a');
    s.pickModel({ provider: 'oino', modelId: 'c' });
    expect(s.choices().fusion).toBeUndefined();
    expect(s.choices().model).toEqual({ provider: 'oino', modelId: 'c' });
    s.pickFusion({ lead: 'oino/a', sidekick: 'oino/b', leadEffort: 'medium', sidekickEffort: 'low' });
    s.pickFusion(undefined);
    expect(s.choices().fusion).toBeUndefined();
  });
});

describe('P0-09: the draft knows this PC before any chat is open', () => {
  it('offers the / commands and models a chat on this PC last listed', async () => {
    const world = createMockHost({ name: 't', hosts: [{ id: 'h', name: 'h', conn: { state: 'connected' } }] });
    setHostClientFactory(world.client);
    rememberCommands('h', [{ name: 'goal', description: 'Set a goal', source: 'extension' }]);
    rememberModels('h', [{ provider: 'oino', id: 'ollamacloud/glm-4.7' }]);
    const s = createRoot(() => createDraftSession('h', () => {}));
    expect(s.state.commands.map((c) => c.name)).toEqual(['goal']);
    const reply = await s.cmd<{ models: Array<{ id: string }> }>({ type: 'get_available_models' });
    expect(reply.models.map((m) => m.id)).toEqual(['ollamacloud/glm-4.7']);
    await s.cmd({ type: 'set_model', provider: 'oino', modelId: 'ollamacloud/glm-4.7' });
    expect(s.choices().model).toEqual({ provider: 'oino', modelId: 'ollamacloud/glm-4.7' });
  });
});

describe('workspace helpers', () => {
  it('finds the home among the roots and walks up', () => {
    expect(homeOf(['/home/oi', '/tmp'])).toBe('/home/oi');
    expect(homeOf(['/srv'])).toBeUndefined();
    expect(parentOf('/home/oi/p')).toBe('/home/oi');
    expect(parentOf('/home')).toBe('/');
  });
});

describe('firstPrompt queue', () => {
  it('is taken exactly once', () => {
    queueFirstPrompt('h', 's1', { text: 'hi' });
    expect(takeFirstPrompt('h', 's1')).toEqual({ text: 'hi' });
    expect(takeFirstPrompt('h', 's1')).toBeUndefined();
    expect(takeFirstPrompt('h', 'other')).toBeUndefined();
  });
});
