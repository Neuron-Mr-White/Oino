// @vitest-environment jsdom
import { render, screen, fireEvent, cleanup } from '@solidjs/testing-library';
import { HashRouter, Route } from '@solidjs/router';
import { vi, it, describe, expect, afterEach } from 'vitest';
import { Pairing, setAddHostOpen } from './Pairing';
import { SettingsProvider } from '../settings/context';
import { HostsProvider } from '../hosts/context';
import { call } from '../lib/ipc';
import { setPlatformOverride } from '../lib/platform';
import { setDesktopOverride } from '../lib/layout';
import { pairPhoneOpen, resetPairPhone, setPairPhoneTransport } from '../lib/pairPhone';

vi.mock('../lib/ipc', () => ({ call: vi.fn() }));

const scanner = vi.hoisted(() => ({
  checkPermissions: vi.fn(),
  requestPermissions: vi.fn(),
  scan: vi.fn(),
  cancel: vi.fn(),
  openAppSettings: vi.fn(),
  Format: { QRCode: 'QR_CODE' },
}));
vi.mock('@tauri-apps/plugin-barcode-scanner', () => scanner);

// A real `unipi-host pair` link (minimal id + relay ticket).
const LINK = 'unipi://pair?t=endpointadn6424ywt6rmkpzqo6zcwxvnry7qwtcnrtzekxaz2wl64ifya6zkaiaenuhi5dqom5c6l3bobztcljrfzzgk3dbpexg4mbonfzg62bonruw42zof4&c=G6R2ANK7QA&n=oi';

afterEach(() => {
  cleanup();
  vi.resetAllMocks();
  setAddHostOpen(false);
  setPlatformOverride(undefined);
  setDesktopOverride(undefined);
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
    // Phone (PH-36): one plain inline error under the field.
    expect((await screen.findByTestId('paste-error')).textContent).toBe("This isn't a Unipi pairing link");
    expect(vi.mocked(call)).not.toHaveBeenCalledWith('pairing_preview', expect.anything(), expect.anything());
  });

  it('desktop app: paste link is the recommended way, QR is for the phone, device defaults to the OS, copy says computer', async () => {
    setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: true });
    setDesktopOverride(true);
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

  it('phone app: Scan QR + Paste, no badges / time captions / SSH placeholder; friendly device name (PH-33)', async () => {
    setPlatformOverride({ mode: 'tauri-mobile', os: 'android', tauri: true });
    (window as Window & { unipiDeviceModel?: string }).unipiDeviceModel = 'sdk_gphone64_x86_64';
    mount();
    setAddHostOpen(true);
    const sheet = await screen.findByTestId('add-host-sheet');
    expect(await screen.findByText('Scan QR code')).toBeInTheDocument();
    expect(sheet.textContent).not.toContain('RECOMMENDED');
    expect(sheet.textContent).not.toContain('~1 min');
    expect(sheet.textContent).not.toContain('SSH');
    fireEvent.click(screen.getByText('Paste pairing link'));
    fireEvent.input(await screen.findByLabelText('Pairing link'), { target: { value: LINK } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(await screen.findByText(/This phone will be able/)).toBeInTheDocument();
    expect(screen.getAllByText('Pair with oi').length).toBeGreaterThan(0);
    expect(screen.getByText('G6R2ANK7QA')).toBeInTheDocument();
    expect(screen.getByDisplayValue('Android phone')).toBeInTheDocument();
    delete (window as Window & { unipiDeviceModel?: string }).unipiDeviceModel;
  });

  it('phone-scan: a refused camera shows a calm note and opens code entry, not an error', async () => {
    setPlatformOverride({ mode: 'tauri-mobile', os: 'android', tauri: true });
    scanner.checkPermissions.mockResolvedValue('prompt');
    scanner.requestPermissions.mockResolvedValue('denied');
    scanner.openAppSettings.mockResolvedValue(undefined);
    mount();
    setAddHostOpen(true);
    fireEvent.click(await screen.findByText('Scan QR code'));
    // PH-34: its own "Can't use the camera" sheet, calm (status, not an alert).
    const note = await screen.findByTestId('camera-off-note');
    expect(note).toHaveAttribute('role', 'status');
    expect(screen.getByTestId('camera-off-sheet').textContent).toContain("Can't use the camera");
    expect(screen.queryByRole('alert')).toBeNull();
    expect(scanner.scan).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Allow the camera in Settings' }));
    expect(scanner.openAppSettings).toHaveBeenCalled();
    fireEvent.click(screen.getByTestId('camera-off-paste'));
    const input = await screen.findByLabelText('Pairing link');
    fireEvent.input(input, { target: { value: LINK } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue' }));
    expect(await screen.findByText(/This phone will be able/)).toBeInTheDocument();
  });

  it('phone-scan: "Enter code instead" on the scanner cancels the scan and opens code entry', async () => {
    setPlatformOverride({ mode: 'tauri-mobile', os: 'android', tauri: true });
    scanner.checkPermissions.mockResolvedValue('granted');
    scanner.scan.mockReturnValue(new Promise(() => undefined));
    scanner.cancel.mockResolvedValue(undefined);
    mount();
    setAddHostOpen(true);
    fireEvent.click(await screen.findByText('Scan QR code'));
    fireEvent.click(await screen.findByRole('button', { name: 'Enter code instead' }));
    expect(scanner.cancel).toHaveBeenCalled();
    expect(await screen.findByLabelText('Pairing link')).toBeInTheDocument();
    expect(screen.queryByTestId('qr-viewfinder')).toBeNull();
  });

  it('UNI-266: the desktop app links to "Pair a phone with this PC"; phones don\'t', async () => {
    resetPairPhone();
    setPairPhoneTransport({ start: async () => undefined, cancel: async () => undefined, startHost: async () => undefined });
    setPlatformOverride({ mode: 'tauri-desktop', os: 'linux', tauri: true });
    setDesktopOverride(true);
    mount();
    setAddHostOpen(true);
    fireEvent.click(await screen.findByTestId('add-host-pair-phone'));
    expect(pairPhoneOpen()).toBe(true);
    await new Promise((r) => setTimeout(r, 50));
    expect(screen.queryByText('Paste pairing link')).not.toBeInTheDocument();
    cleanup();
    resetPairPhone();
    setPlatformOverride({ mode: 'tauri-mobile', os: 'android', tauri: true });
    setDesktopOverride(false);
    mount();
    setAddHostOpen(true);
    await screen.findByText('Scan QR code');
    expect(screen.queryByTestId('add-host-pair-phone')).toBeNull();
    setPairPhoneTransport(undefined);
  });
});
