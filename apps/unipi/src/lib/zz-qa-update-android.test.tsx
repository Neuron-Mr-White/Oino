// @vitest-environment jsdom
// TEMPORARY qa-update check (never committed): the Android feed as
// scripts/release/make-feeds.sh writes it for a fake v1.0.1 → the real
// update.ts parser + the real UpdateBanner/UpdateCard show the notice and
// the APK URL reaches the native download() untouched.
import { render, screen, cleanup, fireEvent, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { readFileSync } from 'node:fs';

vi.mock('@tauri-apps/api/core', () => ({ invoke: vi.fn().mockRejectedValue(new Error('no tauri in tests')), Channel: class {} }));
vi.mock('./board/share', () => ({ openExternal: vi.fn(() => true) }));

const FEED = JSON.parse(readFileSync(process.env.QA_FEED ?? '/tmp/qaupd/local/feeds/app/android/latest.json', 'utf8'));
const download = vi.fn();
(window as unknown as { UniPiUpdate: unknown }).UniPiUpdate = {
  info: () => JSON.stringify({ versionName: '1.0.0', versionCode: 1000000, canInstall: true }),
  download,
  install: () => 'started',
  openInstallSettings: () => {},
};
const fetched: string[] = [];
globalThis.fetch = vi.fn(async (url: string | URL) => {
  fetched.push(String(url));
  return new Response(JSON.stringify(FEED), { status: 200, headers: { 'content-type': 'application/json' } });
}) as typeof fetch;

import { FEED_URL, checkForUpdate, parseFeed, updateState } from './update';
import { setPlatformOverride } from './platform';
import { UpdateBanner, UpdateCard } from '../components/UpdateCard';

afterEach(() => cleanup());

describe('qa-update: android feed v1.0.1 against an installed 1.0.0 (versionCode 1000000)', () => {
  it('constant points at the oino.dev android feed', () => {
    expect(FEED_URL).toBe('https://oino.dev/app/android/latest.json');
  });
  it('the make-feeds output parses (oino.dev APK URL allowed)', () => {
    const f = parseFeed(FEED);
    expect(f?.app?.url).toBe('https://oino.dev/download/UniPi_1.0.1_universal.apk');
    expect(f?.app?.build).toBe(1000001);
  });
  it('check → available → banner + card show it → Update hands the oino.dev URL + sha to the native side', async () => {
    setPlatformOverride('tauri-mobile');
    await checkForUpdate(true);
    expect(fetched[0].startsWith(FEED_URL + '?t=')).toBe(true);
    expect(updateState().kind).toBe('available');
    render(() => (
      <>
        <UpdateBanner onOpen={() => {}} />
        <UpdateCard />
      </>
    ));
    const banner = await screen.findByTestId('update-banner');
    console.log('BANNER:', banner.textContent);
    expect(banner.textContent).toContain('Unipi 1.0.1 (1000001) is ready');
    const card = screen.getByTestId('update-card');
    console.log('CARD:', card.textContent);
    expect(card.textContent).toContain('Version 1.0.1 (1000001) is ready');
    expect(card.textContent).toContain('Installed: 1.0.0 (1000000)');
    fireEvent.click(screen.getAllByText('Update').find((e) => e.closest('.update-card'))!.closest('button')!);
    await waitFor(() => expect(download).toHaveBeenCalled());
    console.log('DOWNLOAD:', JSON.stringify(download.mock.calls[0]));
    expect(download.mock.calls[0]).toEqual([FEED.app.url, FEED.app.sha256, 1000001]);
    setPlatformOverride(undefined);
  });
  it('a feed whose APK is off oino.dev (or http) is ignored', () => {
    expect(parseFeed({ ...FEED, app: { ...FEED.app, url: 'https://github.com/x/UniPi.apk' } })?.app).toBeUndefined();
    expect(parseFeed({ ...FEED, app: { ...FEED.app, url: 'http://oino.dev/download/x.apk' } })?.app).toBeUndefined();
    expect(parseFeed({ ...FEED, app: { ...FEED.app, url: 'https://oino.dev.evil.com/x.apk' } })?.app).toBeUndefined();
  });
});
