import { afterEach, describe, expect, it } from 'vitest';
import { describeProtected, needsTrustDecision, noteTrustDecision, parentChoices, parseTrust, resetTrustDecisions, trustDecisions } from './trust';
import { applySnapshot } from './reducer';
import type { SessionState } from './types';

afterEach(() => resetTrustDecisions());

describe('parseTrust (snapshot project_trust / _oino/trust_get)', () => {
  it('reads state, path and the protected resources', () => {
    expect(parseTrust({ state: 'undecided', path: '/p', protected: ['.oino/mcp.json', { path: '.mcp.json' }] })).toEqual({ state: 'undecided', path: '/p', protected: ['.oino/mcp.json', '.mcp.json'] });
    expect(parseTrust({ state: 'trusted', protected: [] })).toEqual({ state: 'trusted', protected: [] });
    expect(parseTrust({ state: 'denied' })?.state).toBe('untrusted');
    expect(parseTrust({ trusted: true })?.state).toBe('trusted');
  });
  it('an older host sends none', () => {
    expect(parseTrust(undefined)).toBeUndefined();
    expect(parseTrust({})).toBeUndefined();
    expect(parseTrust('x')).toBeUndefined();
  });
});

describe('needsTrustDecision — when the banner shows', () => {
  const undecided = { state: 'undecided' as const, path: '/p/zm', protected: ['.mcp.json'] };
  it('only for an undecided folder that has protected resources', () => {
    expect(needsTrustDecision(undecided)).toBe(true);
    expect(needsTrustDecision({ ...undecided, protected: [] })).toBe(false);
    expect(needsTrustDecision({ ...undecided, state: 'trusted' })).toBe(false);
    expect(needsTrustDecision({ ...undecided, state: 'untrusted' })).toBe(false);
    expect(needsTrustDecision(undefined)).toBe(false);
  });
  it('a decision made in this window hides it at once', () => {
    noteTrustDecision('/p/zm', false);
    expect(trustDecisions()['/p/zm']).toBe(false);
    expect(needsTrustDecision(undecided)).toBe(false);
  });
  it('a decision on a parent folder covers the child; one on a sibling does not', () => {
    noteTrustDecision('/p', true);
    expect(needsTrustDecision(undecided)).toBe(false);
    resetTrustDecisions();
    noteTrustDecision('/p/zm-other', true);
    noteTrustDecision('/pp', true);
    expect(needsTrustDecision(undecided)).toBe(true);
  });
});

describe('describeProtected', () => {
  it('groups the MCP files and names the rest', () => {
    expect(describeProtected(['.oino/mcp.json', '.mcp.json', '.oino/skills', '.pi/extensions/x', 'AGENTS.md'])).toBe('MCP servers (.oino/mcp.json, .mcp.json), skills, extensions, AGENTS');
    expect(describeProtected(['.unipi/config/mcp/mcp-config.json'])).toBe('MCP servers (.unipi/config/mcp/mcp-config.json)');
    expect(describeProtected(['settings.json'])).toBe('settings');
  });
});

describe('parentChoices — Trust parent ▾', () => {
  it('offers up to three parents, never root or a bare home folder', () => {
    expect(parentChoices('/home/me/work/clients/zm')).toEqual(['/home/me/work/clients', '/home/me/work']);
    expect(parentChoices('/home/me/zm')).toEqual([]);
    expect(parentChoices('/srv/a/b/c/d/')).toEqual(['/srv/a/b/c', '/srv/a/b', '/srv/a']);
    expect(parentChoices('/zm')).toEqual([]);
  });
});

describe('the session snapshot carries project_trust', () => {
  const fresh = (): SessionState => ({ hostId: 'h', sessionId: 's', seq: 0, entries: [], byId: {}, leafId: null, entriesRev: 0, liveTools: {}, run: { phase: 'idle', running: false }, queue: { steering: [], followUp: [] }, topics: {}, commands: [], info: {}, dialogs: [], conn: 'connecting', outbox: [] });
  it('is stored on a full snapshot and dropped when the host sends none', () => {
    const s = fresh();
    applySnapshot(s, { session_id: 's', reset: true, seq: 0, project_trust: { state: 'undecided', path: '/p', protected: ['.mcp.json'] } } as never);
    expect(s.projectTrust).toEqual({ state: 'undecided', path: '/p', protected: ['.mcp.json'] });
    applySnapshot(s, { session_id: 's', reset: true, seq: 0 } as never);
    expect(s.projectTrust).toBeUndefined();
  });
});
