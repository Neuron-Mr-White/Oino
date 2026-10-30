// @vitest-environment jsdom
import { render, screen, fireEvent, cleanup } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { vi, it, describe, expect, afterEach } from 'vitest';
import { Pairing, setAddHostOpen } from './Pairing';
import { SettingsProvider } from '../settings/context';
import { HostsProvider } from '../hosts/context';
import { call } from '../lib/ipc';
import { setPlatformOverride } from '../lib/platform';
import { pairPhoneOpen, resetPairPhone, setPairPhoneTransport } from '../lib/pairPhone';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));

// A real `unipi-host pair` link (minimal id + relay ticket).
const LINK = 'unipi://pair?t=endpointadn6424ywt6rmkpzqo6zcwxvnry7qwtcnrtzekxaz2wl64ifya6zkaiaenuhi5dqom5c6l3bobztcljrfzzgk3dbpexg4mbonfzg62bonruw42zof4&c=G6R2ANK7QA&n=oi';

afterEach(() => {
  cleanup();
  vi.resetAllMocks();
  setAddHostOpen(false);
  setPlatformOverride(undefined);
});

function mount() {
  vi.mocked(call).mockImplementation(async (cmd: string) => {
    if (cmd === 'list_hosts') return { ok: true, value: [] };
    if (cmd === 'pairing_preview') return { ok: true, value: { id: 'dbee6b98b4fd1629' } };
    return { ok: true, value: null };
  });
  return render(() => (
    <HashRouter>
      <Route
        path="*"
        component={() => (
          <SettingsProvider>
            <HostsProvider>
              <button onClick={() => setAddHostOpen(true)}>Add a host</button>
              <Pairing />
            </HostsProvider>
          </SettingsProvider>
        )}
      />
    </HashRouter>
  ));
}

describe('Pairing', () => {
  it('paste → Continue opens the confirm sheet and it stays open', async () => {
    mount();
    const trigger = screen.getByText('Add a host');
    trigger.focus();
    fireEvent.click(trigger);
    fireEvent.click(await screen.findByText('Paste pairing link'));
    const input = await screen.findByLabelText('Pairing link');
    fireEvent.input(input, { target: { value: LINK } });
    const cont = screen.getByRole('button', { name: 'Continue' });
    expect(cont).not.toBeDisabled();
    fireEvent.click(cont);
    expect(await screen.findByText(/Pair with/)).toBeInTheDocument();
    expect(vi.mocked(call)).toHaveBeenCalledWith('pairing_preview', { link: LINK }, expect.anything());
    // Focus returning to the page must not dismiss the confirm sheet.
    trigger.focus();
    await new Promise((r) => setTimeout(r, 400));
    expect(screen.getByRole('button', { name: 'Pair' })).toBeInTheDocument();
    expect(screen.queryByText('Paste pairing link')).not.toBeInTheDocument();
  });

  it('Continue is disabled until a link is entered and shows invalid links', async () => {
    mount();
    setAddHostOpen(true);
    fireEvent.click(await screen.findByText('Paste pairing link'));
    expect(screen.getByRole('button', { name: 'Continue' })).toBeDisabled();
    fireEvent.input(screen.getByLabelText('Pairing link'), { target: { value: 'unipi://pair?t=nope&c=1' } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(await screen.findByText('Invalid endpoint ticket')).toBeInTheDocument();
    expect(vi.mocked(call)).not.toHaveBeenCalledWith('pairing_preview', expect.anything(), expect.anything());
  });

  it('desktop app: paste link is the recommended way, QR is for the phone, device defaults to the OS, copy says computer', async () => {
    setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: true });
    mount();
    setAddHostOpen(true);
    const paste = await screen.findByText('Paste pairing link');
    expect(paste.closest('button')!.textContent).toContain('RECOMMENDED');
    const qr = screen.getByText('Pair with QR').closest('button')!;
    expect(qr).toBeDisabled();
    expect(qr.textContent).not.toContain('RECOMMENDED');
    fireEvent.click(paste);
    fireEvent.input(await screen.findByLabelText('Pairing link'), { target: { value: LINK } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(await screen.findByText(/This computer will be able/)).toBeInTheDocument();
    expect(screen.getByDisplayValue('Linux desktop')).toBeInTheDocument();
  });

  it('phone app: QR stays recommended and the device is a Phone', async () => {
    setPlatformOverride({ mode: 'tauri-mobile', os: 'android', tauri: true });
    mount();
    setAddHostOpen(true);
    const qr = (await screen.findByText('Pair with QR')).closest('button')!;
    expect(qr).not.toBeDisabled();
    expect(qr.textContent).toContain('RECOMMENDED');
    fireEvent.click(screen.getByText('Paste pairing link'));
    fireEvent.input(await screen.findByLabelText('Pairing link'), { target: { value: LINK } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(await screen.findByText(/This phone will be able/)).toBeInTheDocument();
    expect(screen.getByDisplayValue('Phone')).toBeInTheDocument();
  });

  it('UNI-266: the desktop app links to "Pair a phone with this PC"; phones don\'t', async () => {
    resetPairPhone();
    setPairPhoneTransport({ start: async () => undefined, cancel: async () => undefined, startHost: async () => undefined });
    setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: true });
    mount();
    setAddHostOpen(true);
    fireEvent.click(await screen.findByTestId('add-host-pair-phone'));
    expect(pairPhoneOpen()).toBe(true);
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText('Paste pairing link')).not.toBeInTheDocument();
    cleanup();
    resetPairPhone();
    setPlatformOverride({ mode: 'tauri-mobile', os: 'android', tauri: true });
    mount();
    setAddHostOpen(true);
    await screen.findByText('Pair with QR');
    expect(screen.queryByTestId('add-host-pair-phone')).toBeNull();
    setPairPhoneTransport(undefined);
  });
});
