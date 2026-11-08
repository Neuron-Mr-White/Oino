// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { Show } from 'solid-js';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HashRouter, Route } from '@solidjs/router';
import { HostsProvider } from '../../hosts/context';
import { SettingsProvider } from '../../settings/context';
import { createMockHost, type MockScenario } from '../../dev/mock/host';
import { setHostClientFactory } from '../../lib/host/factory';
import { setDesktopOverride } from '../../lib/layout';
import { resetHubs } from '../../lib/session/hub';
import { resetHostCollapse } from './HostGroup';
import type { SessionCard } from '../../lib/host/client';
import { StatusSchema, superagentApi, type ProjectRow } from '../superagent/api';
import { LeadModelDialog, leadProjectFor } from './LeadModelDialog';
import { SessionList } from './SessionList';

vi.mock('@tauri-apps/api/event', () => ({ listen: async () => () => {} }));
vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), transformCallback: () => 0 }));
vi.mock('../phone/Sheet', async (importActual) => ({
  ...(await importActual<typeof import('../phone/Sheet')>()),
  SheetFrame: (props: { open: boolean; children: unknown; footer: unknown; label: string }) => (
    <Show when={props.open}><div role="dialog" aria-label={props.label}>{props.children as never}{props.footer as never}</div></Show>
  ),
}));
// The real picker is its own tested surface; here a button picks a fixed model.
vi.mock('../../ui/settings/ModelPickerField', () => ({
  ModelPickerField: (props: { value: string; fallback: string; testId: string; onPick: (k: string | undefined) => void }) => (
    <div>
      <span data-testid="picker-value">{props.value || props.fallback}</span>
      <button type="button" data-testid={props.testId} onClick={() => props.onPick('anthropic/claude-opus-4')}>pick</button>
      <button type="button" data-testid="picker-default" onClick={() => props.onPick(undefined)}>default</button>
    </div>
  ),
}));

const project = (over: Partial<ProjectRow> = {}): ProjectRow => ({ projectId: 'oino', name: 'Oino', autoStart: false, lead: null, ready: 0, root: '/home/you/p/oino', leadModel: null, leadThinking: null, ...over });

afterEach(() => {
  cleanup();
  vi.restoreAllMocks();
  setHostClientFactory(undefined);
  resetHubs();
  setDesktopOverride(undefined);
});

describe('leadProjectFor', () => {
  it('matches the project root to the folder, ignoring trailing slashes', () => {
    const list = [project({ projectId: 'a', root: '/x/a/' }), project({ projectId: 'b', root: '/x/b' })];
    expect(leadProjectFor(list, '/x/a')?.projectId).toBe('a');
    expect(leadProjectFor(list, '/x/b/')?.projectId).toBe('b');
  });
  it('is undefined with no match, no root, no cwd or no projects', () => {
    expect(leadProjectFor([project({ root: '/x/a' })], '/x/other')).toBeUndefined();
    expect(leadProjectFor([project({ root: undefined })], '/x/a')).toBeUndefined();
    expect(leadProjectFor([project()], '')).toBeUndefined();
    expect(leadProjectFor(null, '/x/a')).toBeUndefined();
  });
});

describe('LeadModelDialog', () => {
  const open = (p: ProjectRow, onClose = vi.fn()) => render(() => <LeadModelDialog hostId="pc" project={p} onClose={onClose} />);
  const configure = () => vi.spyOn(superagentApi, 'configureProjectLead').mockResolvedValue({ ok: true, value: { projectId: 'oino', model: null, thinking: null } });

  it('is titled with the project and shows the helper text', () => {
    open(project());
    expect(screen.getByRole('dialog', { name: 'Lead model · Oino' })).toBeTruthy();
    expect(screen.getByText(/A running lead keeps its model until it is restarted/)).toBeTruthy();
    expect(screen.queryByTestId('lead-model-reset')).toBeNull();
  });

  it('prefills from leadModel / leadThinking', () => {
    open(project({ leadModel: 'openai/gpt-5', leadThinking: 'high' }));
    expect(screen.getByTestId('picker-value').textContent).toBe('openai/gpt-5');
    expect((screen.getByTestId('lead-model-thinking') as HTMLSelectElement).value).toBe('high');
    expect(screen.getByTestId('lead-model-reset')).toBeTruthy();
  });

  it('Save sends nulls for the defaults', async () => {
    const spy = configure();
    const onClose = vi.fn();
    open(project(), onClose);
    fireEvent.click(screen.getByTestId('lead-model-save'));
    await waitFor(() => expect(spy).toHaveBeenCalledWith('pc', 'oino', null, null));
    await waitFor(() => expect(onClose).toHaveBeenCalled());
  });

  it('Save sends the picked model and thinking level', async () => {
    const spy = configure();
    open(project());
    fireEvent.click(screen.getByTestId('lead-model-pick'));
    fireEvent.change(screen.getByTestId('lead-model-thinking'), { target: { value: 'xhigh' } });
    fireEvent.click(screen.getByTestId('lead-model-save'));
    await waitFor(() => expect(spy).toHaveBeenCalledWith('pc', 'oino', 'anthropic/claude-opus-4', 'xhigh'));
  });

  it('choosing the default model clears it on save', async () => {
    const spy = configure();
    open(project({ leadModel: 'openai/gpt-5', leadThinking: 'low' }));
    fireEvent.click(screen.getByTestId('picker-default'));
    fireEvent.click(screen.getByTestId('lead-model-save'));
    await waitFor(() => expect(spy).toHaveBeenCalledWith('pc', 'oino', null, 'low'));
  });

  it('Reset sends both null', async () => {
    const spy = configure();
    open(project({ leadModel: 'openai/gpt-5', leadThinking: 'high' }));
    fireEvent.click(screen.getByTestId('lead-model-reset'));
    await waitFor(() => expect(spy).toHaveBeenCalledWith('pc', 'oino', null, null));
  });

  it('Cancel closes without calling the host', () => {
    const spy = configure();
    const onClose = vi.fn();
    open(project(), onClose);
    fireEvent.click(screen.getByTestId('lead-model-cancel'));
    expect(onClose).toHaveBeenCalled();
    expect(spy).not.toHaveBeenCalled();
  });

  it('a failure stays inline and keeps the dialog open', async () => {
    vi.spyOn(superagentApi, 'configureProjectLead').mockResolvedValue({ ok: false, error: { code: 'E_HOST', message: 'Unknown model.', retryable: false } } as never);
    const onClose = vi.fn();
    open(project(), onClose);
    fireEvent.click(screen.getByTestId('lead-model-save'));
    expect((await screen.findByTestId('lead-model-error')).textContent).toBe('Unknown model.');
    expect(onClose).not.toHaveBeenCalled();
  });
});

describe('sidebar project menu: Lead model…', () => {
  beforeEach(() => { localStorage.clear(); resetHostCollapse(); setDesktopOverride(true); });
  const card: SessionCard = { session_id: 'a', name: 'Session a', cwd: '/home/you/p/oino', run_state: 'idle', updated_at: Date.now() - 90_000, message_count: 2 };
  const boot = (projects: ProjectRow[] | undefined) => {
    const scenario: MockScenario = { name: 't', hosts: [{ id: 'this-pc', name: 'workstation', transport: 'local', conn: { state: 'connected' }, cards: [card] }] };
    setHostClientFactory(createMockHost(scenario).client);
    location.hash = '#/';
    const status = vi.spyOn(superagentApi, 'status').mockResolvedValue({ ok: true, value: StatusSchema.parse({ enabled: true, concurrencyCap: 2, leads: [], needsYou: [], review: [], ...(projects ? { projects } : {}) }) });
    render(() => (
      <SettingsProvider><HostsProvider><HashRouter><Route path="*" component={() => <SessionList variant="sidebar" />} /></HashRouter></HostsProvider></SettingsProvider>
    ));
    return status;
  };

  it('shows the item for a board project and opens the prefilled dialog', async () => {
    const status = boot([project({ root: '/home/you/p/oino/', leadModel: 'openai/gpt-5', leadThinking: 'low' })]);
    await waitFor(() => expect(screen.getByTestId('sb-rows-oino')).toBeTruthy());
    fireEvent.click(screen.getByTestId('sb-project-menu-oino'));
    fireEvent.click(await screen.findByTestId('sb-project-lead-model'));
    expect(status).toHaveBeenCalled();
    expect(await screen.findByRole('dialog', { name: 'Lead model · Oino' })).toBeTruthy();
    expect((screen.getByTestId('lead-model-thinking') as HTMLSelectElement).value).toBe('low');
  });

  it('hides the item when no project root matches', async () => {
    const status = boot([project({ root: '/home/you/p/other' })]);
    await waitFor(() => expect(screen.getByTestId('sb-rows-oino')).toBeTruthy());
    fireEvent.click(screen.getByTestId('sb-project-menu-oino'));
    await screen.findByTestId('sb-project-settings');
    await waitFor(() => expect(status).toHaveBeenCalled());
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByTestId('sb-project-lead-model')).toBeNull();
  });

  it('hides the item on a host without per-project controls', async () => {
    boot(undefined);
    await waitFor(() => expect(screen.getByTestId('sb-rows-oino')).toBeTruthy());
    fireEvent.click(screen.getByTestId('sb-project-menu-oino'));
    await screen.findByTestId('sb-project-settings');
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.queryByTestId('sb-project-lead-model')).toBeNull();
  });
});
