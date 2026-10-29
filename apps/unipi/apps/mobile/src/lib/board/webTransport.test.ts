import { afterEach, describe, expect, it, vi } from 'vitest';
import { devTransport, servedByDaemon, TOKEN_HINT, webBoardBase } from './transport';
import { webPc } from './pcs';

type W = { __UNIPI_KB_BASE__?: string };

afterEach(() => {
  delete (window as W).__UNIPI_KB_BASE__;
  vi.unstubAllGlobals();
});

function stubFetch(routes: Record<string, { status: number; body?: unknown }>) {
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  vi.stubGlobal('fetch', async (url: string, init?: RequestInit) => {
    calls.push({ url, init });
    const route = routes[url] ?? { status: 404 };
    return new Response(route.body === undefined ? '' : JSON.stringify(route.body), { status: route.status, headers: { 'content-type': 'application/json' } });
  });
  return calls;
}

describe('web board transport (UNI-117)', () => {
  it('uses the vite /kb proxy in dev, same origin when served by the daemon', () => {
    expect(webBoardBase()).toBe('/kb'); // vitest runs in DEV
    expect(servedByDaemon()).toBe(false);
    (window as W).__UNIPI_KB_BASE__ = '';
    expect(webBoardBase()).toBe('');
    expect(servedByDaemon()).toBe(true);
    (window as W).__UNIPI_KB_BASE__ = 'https://kb.example/';
    expect(webBoardBase()).toBe('https://kb.example');
  });

  it('same-origin: requests the daemon paths with cookies (token gate)', async () => {
    (window as W).__UNIPI_KB_BASE__ = '';
    const calls = stubFetch({ '/api/projects': { status: 200, body: [] } });
    const res = await devTransport.request('dev', { method: 'GET', path: '/api/projects' });
    expect(res.status).toBe(200);
    expect(calls[0]!.url).toBe('/api/projects');
    expect(calls[0]!.init?.credentials).toBe('same-origin');
  });

  it('availability: ok, offline, and a token-gated daemon without the cookie', async () => {
    (window as W).__UNIPI_KB_BASE__ = '';
    stubFetch({ '/api/health': { status: 200, body: { ok: true, version: '3.0.0' } }, '/api/rules': { status: 200, body: {} } });
    expect(await devTransport.availability('dev')).toEqual({ state: 'ok', version: '3.0.0' });

    stubFetch({ '/api/health': { status: 200, body: { ok: true } }, '/api/rules': { status: 401 } });
    expect(await devTransport.availability('dev')).toEqual({ state: 'error', message: TOKEN_HINT });

    stubFetch({ '/api/health': { status: 502 } });
    expect(await devTransport.availability('dev')).toEqual({ state: 'offline' });
  });

  it('a 401 mid-session (cookie expired / token rotated) carries the token hint, not the HTML gate page', async () => {
    (window as W).__UNIPI_KB_BASE__ = '';
    stubFetch({ '/api/projects': { status: 401 } });
    const res = await devTransport.request('dev', { method: 'GET', path: '/api/projects' });
    expect(res.status).toBe(401);
    expect(JSON.parse(new TextDecoder().decode(res.body))).toEqual({ error: TOKEN_HINT });
  });

  it('the synthetic PC is "This computer" when the local daemon serves the page', () => {
    expect(webPc().name).toBe('dev (vite proxy)');
    (window as W).__UNIPI_KB_BASE__ = '';
    expect(webPc()).toEqual({ id: 'dev', name: 'This computer', dev: true });
  });
});
