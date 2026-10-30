import { describe, expect, it } from 'vitest';
import { parsePortInput, pickerState, portLabel, previewUrl, urlPath, viewerBanner, type ForwardInfo } from './forward';

const port = (p: number, extra: Record<string, unknown> = {}) => ({ port: p, bind: '127.0.0.1', http: false, allowed: true, ...extra });
const err = (code: string, message = 'x') => ({ code, message, retryable: false }) as never;

describe('pickerState (UNI-213)', () => {
  it('is loading before the first answer', () => expect(pickerState(undefined).kind).toBe('loading'));
  it('an older host (no ports_list) gets a plain update hint, not the serde error', () => {
    const st = pickerState({ ok: false, error: { code: 'E_SCHEMA', message: 'unknown variant `ports_list`, expected one of `list`, `attach`', retryable: false } });
    expect(st.kind).toBe('error');
    if (st.kind !== 'error') return;
    expect(st.error.message).toBe('This PC runs an older unipi-host without port forwarding.');
    expect(st.error.hint).toBe('Update unipi-host on the PC.');
  });
  it('maps E_FORWARD_DISABLED to the disabled state and other errors to error', () => {
    expect(pickerState({ ok: false, error: err('E_FORWARD_DISABLED') }).kind).toBe('disabled');
    expect(pickerState({ ok: false, error: err('E_TRANSPORT') }).kind).toBe('error');
  });
  it('is empty with no ports, keeping the allow-list for the hint', () => {
    expect(pickerState({ ok: true, value: { ports: [], any: false, allowed: [5173] } })).toEqual({ kind: 'empty', any: false, allowed: [5173] });
  });
  it('splits web servers from other ports and puts allowed ones first', () => {
    const state = pickerState({
      ok: true,
      value: {
        ports: [port(22), port(8080, { http: true, allowed: false }), port(5173, { http: true }), port(3000, { http: true })],
        any: false,
        allowed: [22, 3000, 5173],
      },
    });
    if (state.kind !== 'ports') throw new Error(state.kind);
    expect(state.web.map((p) => p.port)).toEqual([3000, 5173, 8080]);
    expect(state.other.map((p) => p.port)).toEqual([22]);
  });
});

describe('parsePortInput', () => {
  it('accepts plain and :prefixed ports', () => {
    expect(parsePortInput(' 5173 ')).toEqual({ ok: true, port: 5173 });
    expect(parsePortInput(':8080')).toEqual({ ok: true, port: 8080 });
  });
  it.each([['', 'Enter a port number'], ['abc', 'Ports are numbers, like 5173'], ['0', 'Ports go from 1 to 65535'], ['70000', 'Ports go from 1 to 65535'], ['80.5', 'Ports are numbers, like 5173']])(
    'rejects %j',
    (raw, message) => expect(parsePortInput(raw)).toEqual({ ok: false, message }),
  );
});

describe('previewUrl / urlPath', () => {
  it('builds loopback URLs from a path', () => {
    expect(previewUrl(5173, '/')).toBe('http://127.0.0.1:5173/');
    expect(previewUrl(5173, 'docs/a?b=1#c')).toBe('http://127.0.0.1:5173/docs/a?b=1#c');
  });
  it('keeps only the path of a pasted localhost URL', () => {
    expect(previewUrl(41000, 'http://localhost:5173/app/')).toBe('http://127.0.0.1:41000/app/');
    expect(previewUrl(41000, 'http://127.0.0.1:5173')).toBe('http://127.0.0.1:41000/');
  });
  it('shows the path for the URL bar', () => {
    expect(urlPath('http://127.0.0.1:5173/')).toBe('/');
    expect(urlPath('http://127.0.0.1:5173')).toBe('/');
    expect(urlPath('http://127.0.0.1:5173/x/y?z')).toBe('/x/y?z');
  });
});

describe('viewerBanner', () => {
  const info = (extra: Partial<ForwardInfo> = {}): ForwardInfo => ({
    id: 'h:5173', hostId: 'h', remotePort: 5173, localPort: 5173, url: 'http://127.0.0.1:5173/', startedAt: 1,
    active: 0, total: 0, bytesUp: 0, bytesDown: 0, ...extra,
  });
  it('is quiet while healthy', () => expect(viewerBanner(info(), false)).toBeUndefined());
  it('says stopped when the forward is gone', () => expect(viewerBanner(undefined, true)?.kind).toBe('lost'));
  it('reports a closed port', () => {
    const b = viewerBanner(info({ lastError: { error: err('E_NOT_FOUND'), at: 10 } }), false);
    expect(b).toEqual({ kind: 'closed', message: 'Nothing is listening on port 5173 on the PC any more.' });
  });
  it('reports a lost connection', () => {
    expect(viewerBanner(info({ lastError: { error: err('E_TRANSPORT'), at: 10 } }), false)?.kind).toBe('lost');
    expect(viewerBanner(info({ lastError: { error: err('E_TIMEOUT'), at: 10 } }), false)?.kind).toBe('lost');
  });
  it('reports forwarding switched off mid-session', () => {
    expect(viewerBanner(info({ lastError: { error: err('E_FORWARD_DISABLED', 'Port forwarding is off on this PC'), at: 10 } }), false)?.kind).toBe('disabled');
  });
  it('clears once a later connection succeeds', () => {
    expect(viewerBanner(info({ lastError: { error: err('E_NOT_FOUND'), at: 10 }, lastOk: 11 }), false)).toBeUndefined();
  });
});

describe('portLabel', () => {
  it('prefers the process name', () => {
    expect(portLabel(port(1, { process: 'node' }))).toBe('node');
    expect(portLabel(port(1, { http: true }))).toBe('Web server');
    expect(portLabel(port(1))).toBe('Unknown process');
  });
});
