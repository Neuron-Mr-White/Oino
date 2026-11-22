/**
 * Capture mock of the PC's browser (`window.__UNIPI_BROWSER__`, the seam `lib/browser.ts`
 * `transport()` reads): two profiles, a fake job-search page drawn into the frames, and the
 * `browser_status{session_id}` answer `session_profile` that names the chat's agent profile.
 * `agentProfile: undefined` = an older PC that does not know the field. Never part of the app.
 */
import { encodeFrame, type BrowserTransport } from '../../lib/browser';
import type { BrowserProfilesData } from '@unipi/protocol';

const PROFILES: BrowserProfilesData = {
  default: 'default',
  profiles: [
    { name: 'default', created: 1, running: false, cookie_domains: [] },
    { name: 'ace-jobs', created: 2, running: true, engine: 'cloak', stealth: true, cookie_domains: ['linkedin.com', 'indeed.com', 'wellfound.com'] },
  ],
};

/** A fake job-search page (400×300 CSS px) as JPEG bytes. */
async function pageJpeg(profile: string): Promise<Uint8Array> {
  const c = document.createElement('canvas');
  c.width = 800;
  c.height = 600;
  const g = c.getContext('2d')!;
  g.fillStyle = '#f6f7f9';
  g.fillRect(0, 0, 800, 600);
  g.fillStyle = '#0a66c2';
  g.fillRect(0, 0, 800, 56);
  g.fillStyle = '#fff';
  g.font = 'bold 22px sans-serif';
  g.fillText(profile === 'ace-jobs' ? 'Jobs · signed in as ace' : 'Jobs · signed out', 24, 36);
  g.fillStyle = '#1b1f23';
  g.font = '16px sans-serif';
  for (let i = 0; i < 5; i += 1) {
    g.fillStyle = '#fff';
    g.fillRect(24, 84 + i * 96, 752, 80);
    g.fillStyle = '#1b1f23';
    g.font = 'bold 17px sans-serif';
    g.fillText(['Senior Rust engineer', 'Staff platform engineer', 'Solid.js UI lead', 'Developer tools engineer', 'Founding engineer'][i]!, 44, 118 + i * 96);
    g.fillStyle = '#6b7280';
    g.font = '14px sans-serif';
    g.fillText('Remote · posted 2 days ago · Easy apply', 44, 142 + i * 96);
  }
  const blob = await new Promise<Blob>((resolve) => c.toBlob((b) => resolve(b!), 'image/jpeg', 0.8));
  return new Uint8Array(await blob.arrayBuffer());
}

/** Calls the mock saw (tests/captures read `window.__UNIPI_BROWSER_LOG__`). */
export type MockBrowserCall = { op: 'status' | 'open'; hostId: string; sessionId?: string; profile?: string };

export function createMockBrowser(opts: { agentProfile?: string } = {}): BrowserTransport {
  const log: MockBrowserCall[] = [];
  (window as unknown as { __UNIPI_BROWSER_LOG__?: MockBrowserCall[] }).__UNIPI_BROWSER_LOG__ = log;
  return {
    status: async (hostId, sessionId) => {
      log.push({ op: 'status', hostId, sessionId });
      return {
        ok: true,
        value: {
          enabled: true,
          running: true,
          headless: false,
          idle_minutes: 10,
          tabs: [],
          can_show_window: true,
          ...(sessionId && opts.agentProfile ? { session_profile: opts.agentProfile } : {}),
        },
      };
    },
    close: async () => ({ ok: true, value: null }),
    window: async (hostId) => ({ ok: true, value: { enabled: true, running: true, headless: false, idle_minutes: 10, tabs: [] } }),
    profiles: async () => ({ ok: true, value: PROFILES }),
    takeover: async (_h, on) => ({ ok: true, value: { connected: true, driving: false, taken_over: on } }),
    open: async (hostId, args, onMsg, onFrame) => {
      log.push({ op: 'open', hostId, profile: args.profile });
      const profile = args.profile ?? 'default';
      const jpeg = await pageJpeg(profile);
      const tab = { id: 'T1', url: 'https://www.linkedin.com/jobs/search/?keywords=rust', title: 'Rust jobs' };
      setTimeout(() => {
        onMsg({ kind: 'event', event: { kind: 'page', url: tab.url, title: tab.title, loading: false, can_back: false, can_forward: false } });
        onFrame(encodeFrame(1, 800, 600, jpeg));
      }, 50);
      return {
        ok: true,
        value: {
          streamId: 'mock',
          open: { browser: { name: 'CloakBrowser', path: '/c' }, headless: false, target: 'T1', tabs: [tab], engine: 'cloak', stealth: true, profile, agent: { connected: true, driving: false, taken_over: false }, display: 'hidden' },
        },
      };
    },
    cmd: async () => {},
    stop: () => {},
  };
}
