// @vitest-environment jsdom
/** UNI-266: "Pair a phone" dialog states with a fake core (the JSON lines of `unipi-host pair --json`). */
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { encode } from 'uqr';
import { PairPhoneDialog, QrSvg } from './PairPhone';
import { setPlatformOverride } from '../lib/platform';
import {
  closePairPhone,
  countdown,
  errorKind,
  groupCode,
  openPairPhone,
  pairPhoneOpen,
  pairPhoneState,
  reduce,
  resetPairPhone,
  setPairPhoneTransport,
  type PairEvent,
} from '../lib/pairPhone';

vi.mock('../lib/board/share', () => ({ copyText: vi.fn(async () => true) }));
vi.mock('../lib/sharing', () => ({ refreshSharing: vi.fn(async () => ({ ok: true })) }));
import { copyText } from '../lib/board/share';

const LINK = 'unipi://pair?t=endpointadn6424ywt6rmkpzqo6zcwxvnry7qwtcnrtzekxaz2wl64ifya6zkaiaenuhi5dqom5c6l3bobztcljrfzzgk3dbpexg4mbonfzg62bonruw42zof4&c=G6R2ANK7QA&n=oi';

let emit: (e: PairEvent) => void;
let starts: number;
let cancels: number;
let startHost: () => Promise<void>;
let startImpl: (() => Promise<void>) | undefined;

beforeEach(() => {
  resetPairPhone();
  setPlatformOverride('tauri-desktop');
  starts = 0;
  cancels = 0;
  startImpl = undefined;
  startHost = async () => undefined;
  setPairPhoneTransport({
    start: async (onEvent) => {
      starts++;
      emit = onEvent;
      if (startImpl) await startImpl();
    },
    cancel: async () => void cancels++,
    startHost: () => startHost(),
  });
});
afterEach(() => {
  cleanup();
  setPairPhoneTransport(undefined);
  setPlatformOverride(undefined);
  vi.useRealTimers();
  vi.clearAllMocks();
});

const waiting = (secondsLeft = 600): PairEvent => ({ state: 'waiting', link: LINK, code: 'G6R2ANK7QA', expires_at: Math.floor(Date.now() / 1000) + secondsLeft, host: 'oi-desktop' });

describe('pure helpers', () => {
  it('countdown, code grouping, error kinds, reducer', () => {
    expect(countdown(600_000, 0)).toBe('10:00');
    expect(countdown(61_500, 0)).toBe('1:02');
    expect(countdown(0, 5)).toBe('0:00');
    expect(groupCode('G6R2ANK7QA')).toBe('G6R2A NK7QA');
    expect(groupCode('ABC123')).toBe('ABC123');
    expect(errorKind("unipi-host isn't installed on this PC", 'E_NOT_FOUND')).toBe('not_installed');
    expect(errorKind('`unipi-host serve` isn\'t running on this PC — start unipi-host serve first')).toBe('not_running');
    expect(errorKind('x', 'E_UNSUPPORTED')).toBe('too_old');
    expect(errorKind('boom')).toBe('other');
    const w = reduce({ kind: 'starting' }, waiting());
    expect(w.kind).toBe('waiting');
    const p = reduce(w, { state: 'paired', device: 'Pixel 9' });
    expect(p).toEqual({ kind: 'paired', device: 'Pixel 9', host: 'oi-desktop' });
    // A late line after a final state changes nothing.
    expect(reduce(p, { state: 'expired', reason: 'cancelled' })).toBe(p);
  });

  it('the SVG draws exactly the encoded matrix, with a white quiet zone', () => {
    render(() => <QrSvg value={LINK} />);
    const svg = screen.getByTestId('pair-phone-qr');
    const qr = encode(LINK, { ecc: 'M', border: 4 });
    expect(Number(svg.getAttribute('data-modules'))).toBe(qr.size);
    const drawn = new Set([...svg.querySelector('path')!.getAttribute('d')!.matchAll(/M(\d+) (\d+)/g)].map((m) => `${m[1]},${m[2]}`));
    const want = new Set<string>();
    qr.data.forEach((row, y) => row.forEach((dark, x) => dark && want.add(`${x},${y}`)));
    expect(drawn).toEqual(want);
    // Quiet zone: nothing dark in the 4-module border.
    for (const key of drawn) {
      const [x, y] = key.split(',').map(Number);
      expect(x >= 4 && y >= 4 && x < qr.size - 4 && y < qr.size - 4).toBe(true);
    }
    expect(svg.querySelector('rect')!.getAttribute('fill')).toBe('#fff');
  });
});

describe('Pair a phone dialog', () => {
  it('starting → waiting (QR, 10:00, code, Copy link) → paired', async () => {
    render(() => <PairPhoneDialog />);
    openPairPhone();
    expect(await screen.findByTestId('pair-phone-starting')).toBeInTheDocument();
    expect(screen.getByText('Making a pairing code…')).toBeInTheDocument();
    await waitFor(() => expect(starts).toBe(1));
    emit(waiting());
    expect(await screen.findByTestId('pair-phone-qr')).toBeInTheDocument();
    expect(screen.getByTestId('pair-phone-countdown').textContent).toMatch(/(10:00|9:59)/);
    expect(screen.getByTestId('pair-phone-code').textContent).toBe('G6R2A NK7QA');
    fireEvent.click(screen.getByTestId('pair-phone-copy'));
    await waitFor(() => expect(copyText).toHaveBeenCalledWith(LINK));
    expect(await screen.findByText('Copied')).toBeInTheDocument();
    emit({ state: 'paired', device: 'Pixel 9' });
    expect(await screen.findByText('Paired with Pixel 9')).toBeInTheDocument();
    expect(screen.getByText(/It shows up in the phone app now/)).toBeInTheDocument();
    expect(screen.queryByTestId('pair-phone-qr')).toBeNull();
    fireEvent.click(screen.getByTestId('pair-phone-done'));
    expect(pairPhoneOpen()).toBe(false);
    // Nothing to cancel after a pairing.
    expect(cancels).toBe(0);
  });

  it('the countdown runs and an expired code offers a new one', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    render(() => <PairPhoneDialog />);
    openPairPhone();
    await vi.waitFor(() => expect(starts).toBe(1));
    emit(waiting(3));
    await vi.waitFor(() => expect(screen.getByTestId('pair-phone-countdown').textContent).toMatch(/0:0[23]/));
    vi.advanceTimersByTime(4000);
    await vi.waitFor(() => expect(screen.getByTestId('pair-phone-expired')).toBeInTheDocument());
    expect(screen.getByText(/ran out after 10 minutes/)).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('pair-phone-retry'));
    await vi.waitFor(() => expect(starts).toBe(2));
    expect(pairPhoneState().kind).toBe('starting');
  });

  it("expired from the host (another pair replaced it) says why", async () => {
    render(() => <PairPhoneDialog />);
    openPairPhone();
    await waitFor(() => expect(starts).toBe(1));
    emit(waiting());
    emit({ state: 'expired', reason: 'replaced' });
    expect(await screen.findByText(/Another pairing started on this PC/)).toBeInTheDocument();
  });

  it('host not running: the start command, Start unipi-host, then a new attempt', async () => {
    let started = 0;
    startHost = async () => void started++;
    render(() => <PairPhoneDialog />);
    openPairPhone();
    await waitFor(() => expect(starts).toBe(1));
    emit({ state: 'error', message: "`unipi-host serve` isn't running on this PC — start unipi-host serve first", hint: 'Start it: systemctl --user start unipi-host' });
    const err = await screen.findByTestId('pair-phone-error');
    expect(err.dataset.kind).toBe('not_running');
    expect(screen.getByText("unipi-host isn't running")).toBeInTheDocument();
    expect(screen.getByTestId('pair-phone-start-cmd').textContent).toContain('systemctl --user start unipi-host');
    fireEvent.click(screen.getByTestId('pair-phone-start'));
    await waitFor(() => expect(started).toBe(1));
    await waitFor(() => expect(starts).toBe(2));
  });

  it('a failing Start shows the reason and keeps the dialog', async () => {
    startHost = async () => {
      throw { code: 'E_INTERNAL', message: '`systemctl --user start unipi-host` failed: Unit not found', hint: 'Run `unipi-host serve` in a terminal' };
    };
    render(() => <PairPhoneDialog />);
    openPairPhone();
    await waitFor(() => expect(starts).toBe(1));
    emit({ state: 'error', message: "start unipi-host serve first" });
    fireEvent.click(await screen.findByTestId('pair-phone-start'));
    expect(await screen.findByRole('alert')).toHaveTextContent('Unit not found — Run `unipi-host serve`');
    expect(starts).toBe(1);
  });

  it('not installed (the core throws E_NOT_FOUND): the install command for this OS', async () => {
    startImpl = async () => {
      throw { code: 'E_NOT_FOUND', message: "unipi-host isn't installed on this PC", hint: 'Install it: …', retryable: false };
    };
    setPlatformOverride({ mode: 'tauri-desktop', os: 'windows', tauri: true });
    render(() => <PairPhoneDialog />);
    openPairPhone();
    expect(await screen.findByText("unipi-host isn't installed on this PC")).toBeInTheDocument();
    expect(screen.getByTestId('pair-phone-install-cmd').textContent).toContain('irm https://unipi.nrn.one/install.ps1 | iex');
    expect(screen.queryByTestId('pair-phone-start')).toBeNull();
    fireEvent.click(screen.getByTestId('pair-phone-retry'));
    await waitFor(() => expect(starts).toBe(2));
  });

  it('too old a host: update command', async () => {
    render(() => <PairPhoneDialog />);
    openPairPhone();
    await waitFor(() => expect(starts).toBe(1));
    emit({ state: 'error', code: 'E_UNSUPPORTED', message: "This PC's unipi-host is too old to pair from here" });
    expect(await screen.findByText("This PC's unipi-host is too old")).toBeInTheDocument();
    expect(screen.getByTestId('pair-phone-install-cmd').textContent).toContain('install.sh');
  });

  it('closing while waiting cancels the code; late events of that attempt are ignored', async () => {
    render(() => <PairPhoneDialog />);
    openPairPhone();
    await waitFor(() => expect(starts).toBe(1));
    const old = emit;
    old(waiting());
    await screen.findByTestId('pair-phone-qr');
    closePairPhone();
    await waitFor(() => expect(cancels).toBe(1));
    expect(pairPhoneOpen()).toBe(false);
    old({ state: 'paired', device: 'late' });
    expect(pairPhoneState().kind).toBe('idle');
  });

  it('renders nothing on phones and the web', () => {
    setPlatformOverride('tauri-mobile');
    const { container } = render(() => <PairPhoneDialog />);
    openPairPhone();
    expect(container.innerHTML).toBe('');
    expect(pairPhoneOpen()).toBe(false);
    setPlatformOverride('web');
    openPairPhone();
    expect(pairPhoneOpen()).toBe(false);
  });
});
