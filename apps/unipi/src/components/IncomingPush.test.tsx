// @vitest-environment jsdom
/** UNI-214: the "Incoming from <PC>" sheet — every state, all three platforms. */
import { render, screen, waitFor, cleanup, fireEvent } from '@solidjs/testing-library';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { PushItem } from '@unipi/protocol';
import { IncomingPushSheet } from './IncomingPush';
import {
  currentIncoming,
  failureState,
  humanSize,
  offerActions,
  pushStore,
  resetPushStore,
  setPushTransport,
  startPushPoll,
  type PushTransport,
} from '../lib/push';
import { setPlatformOverride } from '../lib/platform';

const apk: PushItem = {
  id: 'p1',
  name: 'app-release.apk',
  size: 12 * 1024 * 1024,
  sha256: 'a'.repeat(64),
  mime: 'application/vnd.android.package-archive',
  path: '/home/me/app-release.apk',
  note: 'fresh build',
  created_at: 1,
};
const pdf: PushItem = { ...apk, id: 'p2', name: 'report.pdf', mime: 'application/pdf', size: 640 * 1024, note: undefined, path: '/home/me/report.pdf' };

type Calls = { done: Array<[string, string, string, string | undefined]>; fetch: string[] };
let calls: Calls;
let fetchImpl: PushTransport['fetch'];
function transport(extra: Partial<PushTransport> = {}): PushTransport {
  return {
    list: async () => ({ enabled: true, host_name: 'coffee', pushes: [] }),
    fetch: (hostId, item, onProgress) => {
      calls.fetch.push(item.id);
      return fetchImpl(hostId, item, onProgress);
    },
    done: async (hostId, id, outcome, detail) => {
      calls.done.push([hostId, id, outcome, detail]);
    },
    saveAs: async () => '/home/me/Downloads/report.pdf',
    open: async () => {},
    ...extra,
  };
}

type Native = { canInstall: boolean; install: string; installed: string[]; settings: number; files: Array<[string, string, string, string]> };
let native: Native;
function installNative() {
  native = { canInstall: true, install: 'started', installed: [], settings: 0, files: [] };
  Object.assign(window, {
    UniPiUpdate: {
      info: () => JSON.stringify({ versionName: '0.3.6', versionCode: 3006, canInstall: native.canInstall }),
      installFile: (p: string) => {
        native.installed.push(p);
        return native.canInstall ? native.install : 'permission';
      },
      openInstallSettings: () => {
        native.settings++;
      },
    },
    UniPiFiles: {
      saveFile: (p: string, n: string, m: string) => {
        native.files.push(['save', p, n, m]);
        return JSON.stringify({ ok: true, uri: 'content://x' });
      },
      openFile: (p: string, n: string, m: string) => {
        native.files.push(['open', p, n, m]);
        return JSON.stringify({ ok: true });
      },
    },
  });
}

function offer(item: PushItem = apk) {
  pushStore().setHost('host-1', 'coffee', [item]);
}
function mount() {
  return render(() => <>{currentIncoming() && <IncomingPushSheet incoming={currentIncoming()!} />}</>);
}
const sheet = () => screen.queryByTestId('push-sheet');

beforeEach(() => {
  calls = { done: [], fetch: [] };
  fetchImpl = async (_h, item, onProgress) => {
    onProgress({ done: item.size / 2, total: item.size });
    return { localPath: `/data/cache/pushes/${item.id}/${item.name}` };
  };
  resetPushStore();
  setPushTransport(transport());
  setPlatformOverride('tauri-mobile');
  installNative();
});
afterEach(() => {
  cleanup();
  setPushTransport(undefined);
  setPlatformOverride(undefined);
  delete (window as { UniPiUpdate?: unknown }).UniPiUpdate;
  delete (window as { UniPiFiles?: unknown }).UniPiFiles;
});

describe('pure rules', () => {
  it('offers Install first for an APK on Android, Open/Save otherwise, Save/Open on desktop, nothing on web', () => {
    expect(offerActions(apk, 'android')).toEqual(['install', 'save']);
    expect(offerActions(pdf, 'android')).toEqual(['open', 'save']);
    expect(offerActions(apk, 'desktop')).toEqual(['save', 'open']);
    expect(offerActions(apk, 'web')).toEqual([]);
    expect(offerActions({ ...pdf, name: 'X.APK', mime: 'application/octet-stream' }, 'android')[0]).toBe('install');
  });
  it('sizes', () => {
    expect(humanSize(12 * 1024 * 1024)).toBe('12.0 MB');
    expect(humanSize(640 * 1024)).toBe('640 KB');
    expect(humanSize(5)).toBe('5 B');
  });
  it('maps a checksum rejection to verify_failed and a revoked path to a clear error', () => {
    expect(failureState('install', { code: 'E_BLOB_REJECTED', message: 'bad', retryable: true, detail: { reason: 'checksum' } }).kind).toBe('verify_failed');
    const gone = failureState('save', { code: 'E_UNAUTHORIZED', message: 'x', retryable: false });
    expect(gone.kind === 'error' && gone.message).toBe('The PC no longer offers this file.');
    const net = failureState('save', { code: 'E_TRANSPORT', message: 'Connection lost', retryable: true });
    expect(net.kind === 'error' && net.message).toBe('Connection lost');
  });
});

describe('Incoming push sheet (UNI-214)', () => {
  it('pending: shows the PC, name, size, note and Install / Save / Dismiss for an APK', async () => {
    offer();
    mount();
    expect(await screen.findByText('Incoming from coffee')).toBeTruthy();
    expect(screen.getByText('app-release.apk')).toBeTruthy();
    expect(screen.getByText('12.0 MB · from coffee')).toBeTruthy();
    expect(screen.getByText('“fresh build”')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Install' }).className).toContain('pair-primary');
    expect(screen.getByRole('button', { name: 'Save' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Dismiss' })).toBeTruthy();
  });

  it('downloading shows progress, then Install hands the verified file to the installer and tells the PC', async () => {
    let finish!: () => void;
    fetchImpl = (_h, item, onProgress) =>
      new Promise((resolve) => {
        onProgress({ done: 3 * 1024 * 1024, total: item.size });
        finish = () => resolve({ localPath: '/data/cache/pushes/p1/app-release.apk' });
      });
    offer();
    mount();
    fireEvent.click(await screen.findByRole('button', { name: 'Install' }));
    const bar = await screen.findByRole('progressbar');
    expect(bar.getAttribute('aria-valuenow')).toBe('25');
    expect(screen.getByText('3.0 MB of 12.0 MB · 25%')).toBeTruthy();
    expect((screen.getByRole('button', { name: 'Downloading…' }) as HTMLButtonElement).disabled).toBe(true);
    finish();
    expect(await screen.findByTestId('push-done')).toBeTruthy();
    expect(screen.getByText(/installer is open/)).toBeTruthy();
    expect(native.installed).toEqual(['/data/cache/pushes/p1/app-release.apk']);
    expect(calls.done).toEqual([['host-1', 'p1', 'installed', 'system installer opened']]);
    fireEvent.click(screen.getByRole('button', { name: 'Done' }));
    await waitFor(() => expect(sheet()).toBeNull());
    expect(calls.done.length).toBe(1);
  });

  it("Android's installer taking focus does not close the sheet (it stays to show 'installer is open')", async () => {
    // Seen on the Redmi: the system installer opening over the app moved focus out of the sheet and
    // corvu's close-on-outside-focus dismissed it mid-install.
    offer();
    mount();
    fireEvent.click(await screen.findByRole('button', { name: 'Install' }));
    expect(await screen.findByTestId('push-done')).toBeTruthy();
    const outside = document.createElement('button');
    document.body.append(outside);
    outside.focus();
    fireEvent.focusIn(outside);
    window.dispatchEvent(new FocusEvent('blur'));
    await new Promise((r) => setTimeout(r, 50));
    expect(sheet()).not.toBeNull();
    expect(screen.getByText(/installer is open/)).toBeTruthy();
    outside.remove();
  });

  it('verify fail: explains, installs nothing, Try again re-downloads, Dismiss reports failed', async () => {
    let attempt = 0;
    fetchImpl = async () => {
      attempt++;
      throw { code: 'E_BLOB_REJECTED', message: 'The file didn’t arrive intact (checksum mismatch).', retryable: true, detail: { reason: 'checksum' } };
    };
    offer();
    mount();
    fireEvent.click(await screen.findByRole('button', { name: 'Install' }));
    expect(await screen.findByTestId('push-verify-failed')).toBeTruthy();
    expect(screen.getByText('The file didn’t arrive intact')).toBeTruthy();
    expect(native.installed).toEqual([]);
    fireEvent.click(screen.getByRole('button', { name: 'Try again' }));
    await waitFor(() => expect(attempt).toBe(2));
    fireEvent.click(await screen.findByRole('button', { name: 'Dismiss' }));
    await waitFor(() => expect(sheet()).toBeNull());
    expect(calls.done[0].slice(0, 3)).toEqual(['host-1', 'p1', 'failed']);
  });

  it('permission missing: explains, Open settings opens Android’s switch, retry installs once allowed', async () => {
    native.canInstall = false;
    offer();
    mount();
    fireEvent.click(await screen.findByRole('button', { name: 'Install' }));
    expect(await screen.findByTestId('push-permission')).toBeTruthy();
    expect(screen.getByText('Allow Unipi to install apps')).toBeTruthy();
    expect(native.installed, 'never opened the installer without the permission').toEqual([]);
    fireEvent.click(screen.getByRole('button', { name: 'Open settings' }));
    expect(native.settings).toBe(1);
    // Still not allowed: retry stays on the permission screen.
    fireEvent.click(screen.getByRole('button', { name: /I’ve allowed it/ }));
    expect(screen.getByTestId('push-permission')).toBeTruthy();
    native.canInstall = true;
    fireEvent.click(screen.getByRole('button', { name: /I’ve allowed it/ }));
    expect(await screen.findByTestId('push-done')).toBeTruthy();
    expect(calls.fetch, 'no second download').toEqual(['p1']);
    expect(native.installed.length).toBe(1);
  });

  it('dismissed: closes, tells the PC, and is not offered again by the next poll', async () => {
    offer();
    mount();
    fireEvent.click(await screen.findByRole('button', { name: 'Dismiss' }));
    await waitFor(() => expect(sheet()).toBeNull());
    expect(calls.done).toEqual([['host-1', 'p1', 'dismissed', undefined]]);
    offer();
    expect(currentIncoming()).toBeUndefined();
  });

  it('non-APK on Android: Open hands it to "Open with…", Save goes to Downloads/unipi', async () => {
    offer(pdf);
    mount();
    expect((await screen.findByRole('button', { name: 'Open' })).className).toContain('pair-primary');
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByText('Saved to Downloads/unipi/report.pdf')).toBeTruthy();
    expect(native.files[0]).toEqual(['save', '/data/cache/pushes/p2/report.pdf', 'report.pdf', 'application/pdf']);
    expect(calls.done[0].slice(0, 3)).toEqual(['host-1', 'p2', 'saved']);
  });

  it('an old APK without the new bridge methods says to update instead of failing silently', async () => {
    (window as unknown as { UniPiUpdate: Record<string, unknown> }).UniPiUpdate.installFile = undefined;
    offer();
    mount();
    fireEvent.click(await screen.findByRole('button', { name: 'Install' }));
    expect(await screen.findByTestId('push-error')).toBeTruthy();
    expect(screen.getByText(/can’t install pushed apps/)).toBeTruthy();
  });

  it('desktop: Save to… (native dialog) and cancelling the dialog returns to the offer', async () => {
    setPlatformOverride('tauri-desktop');
    let picked: string | null = null;
    setPushTransport(transport({ saveAs: async () => picked }));
    offer(pdf);
    mount();
    expect((await screen.findByRole('button', { name: 'Save' })).className).toContain('pair-primary');
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    await waitFor(() => expect(screen.getByRole('button', { name: 'Save' })).toBeTruthy());
    expect(calls.done).toEqual([]);
    picked = '/home/me/Documents/report.pdf';
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));
    expect(await screen.findByText('Saved to /home/me/Documents/report.pdf')).toBeTruthy();
    expect(calls.done[0].slice(0, 3)).toEqual(['host-1', 'p2', 'saved']);
  });

  it('desktop: "Saved to …" stays after the PC drops the push (the poll that follows push_done no longer lists it)', async () => {
    setPlatformOverride('tauri-desktop');
    let release: () => void = () => {};
    setPushTransport(
      transport({
        saveAs: async () => '/home/me/Downloads/report.pdf',
        // The PC answers push_done slowly; meanwhile the long-poll returns without the push.
        done: async (hostId, id, outcome, detail) => {
          calls.done.push([hostId, id, outcome, detail]);
          pushStore().setHost('host-1', 'coffee', []);
          await new Promise<void>((r) => (release = r));
        },
      }),
    );
    offer(pdf);
    mount();
    fireEvent.click(await screen.findByRole('button', { name: 'Save' }));
    expect(await screen.findByText('Saved to /home/me/Downloads/report.pdf')).toBeTruthy();
    release();
    await waitFor(() => expect(calls.done[0]?.[2]).toBe('saved'));
    expect(screen.getByText('Saved to /home/me/Downloads/report.pdf')).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Done' })).toBeTruthy();
  });

  it('web mode: a graceful notice, no actions but Dismiss, and the poller never starts', async () => {
    setPlatformOverride('web');
    const list = vi.fn();
    setPushTransport(transport({ list }));
    const stop = startPushPoll(() => [{ id: 'host-1', name: 'coffee' }]);
    stop();
    expect(list).not.toHaveBeenCalled();
    offer();
    mount();
    expect(await screen.findByTestId('push-web')).toBeTruthy();
    expect(screen.queryByRole('button', { name: 'Install' })).toBeNull();
    expect(screen.getByRole('button', { name: 'Dismiss' })).toBeTruthy();
  });

  it('a revoked push (PC cancelled / push switched off) shows a clear error', async () => {
    fetchImpl = async () => {
      throw { code: 'E_UNAUTHORIZED', message: 'That path isn’t available to the phone', retryable: false };
    };
    offer();
    mount();
    fireEvent.click(await screen.findByRole('button', { name: 'Save' }));
    expect(await screen.findByText('The PC no longer offers this file.')).toBeTruthy();
    expect(screen.getByText(/Push it again/)).toBeTruthy();
  });
});

describe('poller + store', () => {
  it('long-polls each paired PC and surfaces what it returns; drops items the PC no longer lists', async () => {
    setPlatformOverride('tauri-mobile');
    const seen: Array<[string, number]> = [];
    let round = 0;
    setPushTransport(
      transport({
        list: async (hostId, wait) => {
          seen.push([hostId, wait]);
          round++;
          if (round === 1) return { enabled: true, host_name: 'coffee', pushes: [apk] };
          await new Promise((r) => setTimeout(r, 1000));
          return { enabled: true, host_name: 'coffee', pushes: [] };
        },
      }),
    );
    const stop = startPushPoll(() => [{ id: 'host-1', name: 'coffee' }], 25);
    await waitFor(() => expect(currentIncoming()?.item.id).toBe('p1'));
    expect(currentIncoming()?.hostName).toBe('coffee');
    expect(seen[0]).toEqual(['host-1', 25]);
    stop();
    pushStore().setHost('host-1', 'coffee', []);
    expect(currentIncoming()).toBeUndefined();
  });

  it('chat-vanish: a long-poll that answers at once never spins (at most one round a second)', async () => {
    setPlatformOverride('tauri-mobile');
    let rounds = 0;
    setPushTransport(transport({ list: async () => (rounds++, { enabled: true, host_name: 'coffee', pushes: [apk] }) }));
    const stop = startPushPoll(() => [{ id: 'host-1', name: 'coffee' }], 25);
    await new Promise((r) => setTimeout(r, 1_500));
    stop();
    // Before: thousands of rounds (and push-layer renders) in 1.5 s — the webview froze.
    expect(rounds).toBeLessThanOrEqual(3);
  });

  it('push off on a PC: nothing shown', () => {
    pushStore().setHost('host-1', 'coffee', []);
    expect(currentIncoming()).toBeUndefined();
  });
});
