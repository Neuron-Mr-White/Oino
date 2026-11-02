import { describe, expect, it } from 'vitest';
import { isNewer, newerVersion, parseFeed } from './update';

const good = { app: { version: '0.2.0', build: 2003, url: 'https://oino.dev/releases/app/unipi-0.2.0-2003.apk', sha256: 'a'.repeat(64), size: 10 } };

describe('update feed', () => {
  it('accepts a well-formed feed from oino.dev only', () => {
    expect(parseFeed(good)?.app?.build).toBe(2003);
    expect(parseFeed({ app: { ...good.app, url: 'https://evil.example/x.apk' } })).toBeNull();
    expect(parseFeed({ app: { ...good.app, sha256: 'nothex' } })).toBeNull();
    expect(parseFeed({ app: { ...good.app, build: '2003' } })).toBeNull();
    expect(parseFeed('garbage')).toBeNull();
  });
  it('offers only a strictly higher build', () => {
    const feed = parseFeed(good);
    expect(isNewer(feed, { versionName: '0.1.0', versionCode: 1000, canInstall: true })?.build).toBe(2003);
    expect(isNewer(feed, { versionName: '0.2.0', versionCode: 2003, canInstall: true })).toBeNull();
    expect(isNewer(feed, { versionName: '0.3.0', versionCode: 3000, canInstall: true })).toBeNull();
    expect(isNewer(feed, null)).toBeNull();
  });
  it('parses the 0.4.4+ feed (desktop, ios, more host platforms) the same way', () => {
    const sha = 'c'.repeat(64);
    const feed = {
      app: { ...good.app, ios: { version: '0.4.4', url: 'https://oino.dev/releases/ios/0.4.4/UniPi-0.4.4.ipa', page: 'https://oino.dev/ios', size: 9 } },
      host: { version: '0.4.4', files: { 'linux-x64': { url: 'https://oino.dev/x', sha256: sha }, 'darwin-arm64': { url: 'https://oino.dev/y', sha256: sha }, 'windows-x64': { url: 'https://oino.dev/z.exe', sha256: sha } } },
      desktop: { version: '0.4.4', platforms: { 'linux-x86_64': { url: 'https://oino.dev/a.AppImage', signature: 'sig' } } },
      downloads: { android: 'https://oino.dev/download/android' },
      install: 'curl -fsSL https://oino.dev/install.sh | sh',
    };
    const parsed = parseFeed(feed);
    expect(parsed?.app?.build).toBe(2003);
    expect(parsed?.ios?.page).toBe('https://oino.dev/ios');
    expect(parseFeed({ app: { ...good.app, ios: { ...feed.app.ios, page: 'https://evil.example/ios' } } })?.ios).toBeUndefined();
    expect(parseFeed({ app: { ...good.app, ios: { ...feed.app.ios, version: 'x' } } })?.app).toBeTruthy();
  });
  it('compares X.Y.Z numerically', () => {
    expect(newerVersion('0.4.10', '0.4.9')).toBe(true);
    expect(newerVersion('0.4.4', '0.4.4')).toBe(false);
    expect(newerVersion('0.4.3', '0.4.4')).toBe(false);
    expect(newerVersion('nope', '0.4.4')).toBe(false);
  });
});
