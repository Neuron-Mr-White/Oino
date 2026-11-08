// @vitest-environment jsdom
import { cleanup, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { superagentApi, StatusSchema, type NeedsItem } from './api';
import { countNeeds } from './needs';
import { SuperagentRailIndicator } from './RailIndicator';
import { setPlatformOverride } from '../../lib/platform';

vi.mock('../shell/HostSwitcher', () => ({ useSelectedHost: () => () => ({ id: 'pc' }) }));

const needs = (over: Partial<NeedsItem>): NeedsItem => ({ projectId: 'p', taskId: 'T-1', status: 'blocked', ...over });
const approval = (n: number) => ({ projectId: 'p', taskId: `T-${n}`, status: 'blocked', workerSessionId: 'w', requestId: `r-${n}` });
const lead = (n: number) => ({ projectId: 'p', sessionId: `s-${n}`, autonomy: 'high', ticketIds: [] });
const status = (leads: object[], needsYou: object[]) => StatusSchema.parse({ enabled: true, concurrencyCap: 2, leads, needsYou });

describe('countNeeds', () => {
  it('counts approval requests', () => expect(countNeeds([needs({ requestId: 'r', workerSessionId: 'w' })])).toBe(1));
  it('counts blocked tickets with a question', () => expect(countNeeds([needs({ question: 'EUR or USD?' })])).toBe(1));
  it('ignores blocked tickets without a question', () => expect(countNeeds([needs({ question: '  ' }), needs({ question: null }), needs({})])).toBe(0));
  it('ignores a question on a ticket that is not blocked', () => expect(countNeeds([needs({ status: 'in_progress', question: 'Hm?' })])).toBe(0));
  it('handles no data', () => expect(countNeeds(undefined)).toBe(0));
});

describe('SuperagentRailIndicator', () => {
  beforeEach(() => setPlatformOverride('tauri-desktop'));
  afterEach(() => { cleanup(); vi.restoreAllMocks(); setPlatformOverride(undefined); });
  const mockStatus = (value: ReturnType<typeof status>) => vi.spyOn(superagentApi, 'status').mockResolvedValue({ ok: true, value });

  it('shows a count pill when something waits on the owner, even with leads running', async () => {
    mockStatus(status([lead(1)], [approval(1), approval(2)]));
    render(() => <SuperagentRailIndicator />);
    const pill = await screen.findByTestId('rail-superagent-needs');
    expect(pill.textContent).toBe('2');
    expect(pill.getAttribute('aria-label')).toBe('2 waiting for you');
    expect(pill.getAttribute('title')).toBe('2 waiting for you');
    expect(screen.queryByTestId('rail-superagent-live')).toBeNull();
  });

  it('shows a pulsing dot while leads run and nothing waits', async () => {
    mockStatus(status([lead(1), lead(2)], []));
    render(() => <SuperagentRailIndicator />);
    const dot = await screen.findByTestId('rail-superagent-live');
    expect(dot.getAttribute('title')).toBe('2 leads running');
    expect(screen.queryByTestId('rail-superagent-needs')).toBeNull();
  });

  it('shows nothing when idle', async () => {
    const spy = mockStatus(status([], []));
    render(() => <SuperagentRailIndicator />);
    await waitFor(() => expect(spy).toHaveBeenCalled());
    await Promise.resolve();
    expect(screen.queryByTestId('rail-superagent-needs')).toBeNull();
    expect(screen.queryByTestId('rail-superagent-live')).toBeNull();
  });

  it('shows nothing when the status is unavailable', async () => {
    const spy = vi.spyOn(superagentApi, 'status').mockRejectedValue(new Error('offline'));
    render(() => <SuperagentRailIndicator />);
    await waitFor(() => expect(spy).toHaveBeenCalled());
    await Promise.resolve();
    expect(screen.queryByTestId('rail-superagent-needs')).toBeNull();
    expect(screen.queryByTestId('rail-superagent-live')).toBeNull();
  });
});
