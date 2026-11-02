// @vitest-environment jsdom
/** UNI-248: the off card — ask flow on another PC, the switch on this computer. */
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { FeatureOff } from './FeatureOff';
import { setPlatformOverride } from '../lib/platform';
import { loadLocalHost, setSharingTransport, SharingStatusSchema, type FeatureRequestItem, type SharingTransport } from '../lib/sharing';

const Cmd = (p: { command: string }) => <code data-testid="cmd">{p.command}</code>;
const item = (extra: Partial<FeatureRequestItem> = {}): FeatureRequestItem => ({
  id: 'r1', feature: 'browser', scope: {}, device_name: 'Pixel', status: 'pending', created_at: Date.now(), ...extra,
});
const statusOk = SharingStatusSchema.parse({
  version: '0.4.1', host_dir: '/tmp/qa-248',
  forward: { enabled: true, any: true, ports: [] }, browser: { enabled: false, headless: true, idle_minutes: 10 },
  desktop: { enabled: true, view_only: true, display: 'x11' }, push: { enabled: false },
});

let requests: FeatureRequestItem[] = [];
let t: SharingTransport & { asks: unknown[]; changes: unknown[] };
function transport(extra: Partial<SharingTransport> = {}) {
  t = {
    asks: [],
    changes: [],
    localHost: async () => ({ ok: true, value: { hostDir: '/tmp/qa-248', hostId: 'local-id' } }),
    status: async () => ({ ok: true, value: statusOk }),
    change: async (c) => {
      t.changes.push(c);
      return { ok: true, value: statusOk };
    },
    ask: async (hostId, feature, scope, note) => {
      t.asks.push({ hostId, feature, scope, note });
      return { ok: true, value: item({ feature, scope: scope ?? {}, note }) };
    },
    requests: async () => ({ ok: true, value: requests }),
    ...extra,
  } as typeof t;
  setSharingTransport(t);
}

beforeEach(() => {
  requests = [];
  setPlatformOverride('tauri-mobile');
  transport();
});
afterEach(() => {
  cleanup();
  vi.useRealTimers();
  setPlatformOverride(undefined);
  setSharingTransport(undefined);
});

describe('ask the PC (phone / other PC)', () => {
  it('asks with the chosen scope and note, then waits; the poll shows the answer', async () => {
    const changed = vi.fn();
    render(() => <FeatureOff feature="browser" hostId="h1" hostName="coffee" CommandBox={Cmd} onChanged={changed} />);
    const card = await screen.findByTestId('share-ask-browser');
    expect(card.textContent).toContain('The remote browser is off on this PC');
    expect(card.textContent).toContain('Nothing changes until they say yes');
    fireEvent.click(screen.getByLabelText(/Show its window on the PC/));
    fireEvent.input(screen.getByLabelText('Note for the PC'), { target: { value: 'quick look' } });
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    fireEvent.click(screen.getByTestId('share-ask'));
    await waitFor(() => expect(card.dataset.state).toBe('pending'));
    expect(t.asks[0]).toEqual({ hostId: 'h1', feature: 'browser', scope: { headed: true }, note: 'quick look' });
    expect(card.textContent).toContain('Waiting for coffee');
    expect(card.textContent).toContain('window shown on the PC');
    // The PC approves for an hour: the next poll shows it and re-checks the panel.
    requests = [item({ status: 'approved', decided_at: Date.now(), until: Date.now() + 3_600_000 })];
    await vi.advanceTimersByTimeAsync(4100);
    await waitFor(() => expect(card.dataset.state).toBe('approved'));
    expect(card.textContent).toContain('coffee said yes');
    expect(card.textContent).toMatch(/for (60|59) more min/);
    expect(changed).toHaveBeenCalled();
  });

  it('denied → Ask again; expired explains itself', async () => {
    requests = [item({ status: 'denied', decided_at: Date.now() })];
    render(() => <FeatureOff feature="browser" hostId="h1" hostName="coffee" CommandBox={Cmd} onChanged={() => {}} />);
    const card = await screen.findByTestId('share-ask-browser');
    await waitFor(() => expect(card.dataset.state).toBe('denied'));
    expect(card.textContent).toContain('coffee said no');
    fireEvent.click(screen.getByText('Ask again'));
    expect(card.dataset.state).toBe('idle');
    cleanup();
    requests = [item({ status: 'expired', decided_at: Date.now() })];
    render(() => <FeatureOff feature="browser" hostId="h1" hostName="coffee" CommandBox={Cmd} onChanged={() => {}} />);
    await waitFor(() => expect(screen.getByTestId('share-ask-browser').dataset.state).toBe('expired'));
    expect(screen.getByTestId('share-ask-browser').textContent).toContain('Nobody answered');
  });

  it('a stale answer from long ago is not shown; a pending one is', async () => {
    requests = [item({ status: 'denied', created_at: 1, decided_at: 1 })];
    render(() => <FeatureOff feature="browser" hostId="h1" hostName="coffee" CommandBox={Cmd} onChanged={() => {}} />);
    await new Promise((r) => setTimeout(r, 20));
    expect(screen.getByTestId('share-ask-browser').dataset.state).toBe('idle');
  });

  it('forward: "Only these ports" validates before asking', async () => {
    render(() => <FeatureOff feature="forward" hostId="h1" hostName="coffee" CommandBox={Cmd} onChanged={() => {}} />);
    await screen.findByTestId('share-ask-forward');
    fireEvent.click(screen.getByRole('radio', { name: 'Only these ports' }));
    fireEvent.click(screen.getByTestId('share-ask'));
    expect((await screen.findByRole('alert')).textContent).toContain('at least one port');
    expect(t.asks).toHaveLength(0);
    fireEvent.input(screen.getByLabelText('Ports'), { target: { value: '5173, 8080' } });
    fireEvent.click(screen.getByTestId('share-ask'));
    await waitFor(() => expect(t.asks).toHaveLength(1));
    expect(t.asks[0]).toMatchObject({ scope: { ports: [5173, 8080] } });
  });

  it('an old host (no feature_request op) gets the command instead', async () => {
    transport({ requests: async () => ({ ok: false, error: { code: 'E_SCHEMA', message: 'unknown variant `feature_requests`', retryable: false } }) });
    render(() => <FeatureOff feature="push" hostId="h1" hostName="coffee" CommandBox={Cmd} onChanged={() => {}} />);
    await waitFor(() => expect(screen.getByTestId('share-ask-push').dataset.state).toBe('old-host'));
    expect(screen.getByTestId('cmd').textContent).toBe('unipi-oino-host push enable');
  });

  it('a failed ask shows the error and keeps the form', async () => {
    transport({ ask: async () => ({ ok: false, error: { code: 'E_TRANSPORT', message: "Can't reach coffee", retryable: true } }) });
    render(() => <FeatureOff feature="browser" hostId="h1" hostName="coffee" CommandBox={Cmd} onChanged={() => {}} />);
    await screen.findByTestId('share-ask-browser');
    fireEvent.click(screen.getByTestId('share-ask'));
    expect((await screen.findByRole('alert')).textContent).toContain("Can't reach coffee");
    expect(screen.getByTestId('share-ask')).toBeTruthy();
  });
});

describe('this computer (desktop app)', () => {
  it('the switch turns it on through the CLI with the options', async () => {
    setPlatformOverride('tauri-desktop');
    transport();
    await loadLocalHost();
    const changed = vi.fn();
    render(() => <FeatureOff feature="browser" hostId="local-id" hostName="coffee" CommandBox={Cmd} onChanged={changed} />);
    const card = await screen.findByTestId('share-local-browser');
    expect(card.textContent).toContain('on this computer');
    fireEvent.click(screen.getByLabelText(/Show its window on the PC/));
    fireEvent.click(screen.getByRole('switch'));
    await waitFor(() => expect(changed).toHaveBeenCalled());
    expect(t.changes).toEqual([{ kind: 'browser_enable', headed: true }]);
  });

  it('another PC from the desktop app still asks; a missing host says to reinstall Unipi', async () => {
    setPlatformOverride('tauri-desktop');
    transport({ change: async () => ({ ok: false, error: { code: 'E_NOT_FOUND', message: 'This Unipi install is missing its agent host', retryable: false } }) });
    await loadLocalHost();
    render(() => (
      <>
        <FeatureOff feature="forward" hostId="other" hostName="laptop" CommandBox={Cmd} onChanged={() => {}} />
        <FeatureOff feature="forward" hostId="local-id" hostName="coffee" CommandBox={Cmd} onChanged={() => {}} />
      </>
    ));
    expect(await screen.findByTestId('share-ask-forward')).toBeTruthy();
    fireEvent.click(screen.getByRole('switch'));
    expect((await screen.findByRole('alert')).textContent).toContain('missing its agent host');
    expect(screen.getByRole('alert').textContent).toContain('Reinstall or update Unipi');
    expect(screen.queryAllByTestId('cmd').some((c) => c.textContent?.includes('install.sh'))).toBe(false);
  });
});
