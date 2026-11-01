import { beforeEach, describe, expect, it } from 'vitest';
import {
  bindTabTerm,
  clearTermSnapshot,
  cwdLeaf,
  nextTerminalTitle,
  normalizeTermCwd,
  parseTermEvent,
  pushTermHistory,
  readTermHistory,
  readTermSnapshot,
  saveTermSnapshot,
  SNAPSHOT_MAX_CHARS,
  tabTermBinding,
  TermProcessTracker,
  unbindTabTerm,
} from './term';

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
});

describe('terminal tab naming (Z-105)', () => {
  it('normalises the cwd sent to term_open', () => {
    expect(normalizeTermCwd(undefined)).toBe('~');
    expect(normalizeTermCwd('  ')).toBe('~');
    expect(normalizeTermCwd('relative/dir')).toBe('~');
    expect(normalizeTermCwd('~/p')).toBe('~/p');
    expect(normalizeTermCwd('/a/b/')).toBe('/a/b/');
    expect(normalizeTermCwd('C:\\work')).toBe('C:\\work');
  });

  it('names after the folder leaf', () => {
    expect(cwdLeaf('/home/oi/oino')).toBe('oino');
    expect(cwdLeaf('/home/oi/oino/')).toBe('oino');
    expect(cwdLeaf('~')).toBe('~');
    expect(cwdLeaf('/')).toBe('/');
  });

  it('no "1", then the lowest free number', () => {
    expect(nextTerminalTitle('/x/oino', [])).toBe('oino');
    expect(nextTerminalTitle('/x/oino', ['oino'])).toBe('oino 2');
    expect(nextTerminalTitle('/x/oino', ['oino', 'oino 2', 'oino 3'])).toBe('oino 4');
    expect(nextTerminalTitle('/x/oino', ['oino', 'oino 3'])).toBe('oino 2');
    expect(nextTerminalTitle('/x/oino', ['oino 2'])).toBe('oino');
    expect(nextTerminalTitle('/x/oino', ['oino 02', 'oinox', 'other'])).toBe('oino');
  });
});

describe('terminal wire events', () => {
  it('parses title / exit / closed and rejects junk', () => {
    expect(parseTermEvent({ type: 'exit', code: null })).toEqual({ ok: true, event: { type: 'exit', code: null } });
    expect(parseTermEvent({ type: 'title', title: 'vim' }).ok).toBe(true);
    expect(parseTermEvent({ type: 'nope' })).toMatchObject({ ok: false, error: { code: 'E_SCHEMA' } });
  });
});

describe('re-attach bindings and snapshots', () => {
  it('binds a tab to its host terminal and forgets it', () => {
    bindTabTerm('tab1', { termId: 't1', hostId: 'h', cwd: '/p', shell: 'bash' });
    expect(tabTermBinding('tab1')).toEqual({ termId: 't1', hostId: 'h', cwd: '/p', shell: 'bash' });
    unbindTabTerm('tab1');
    expect(tabTermBinding('tab1')).toBeUndefined();
    expect(localStorage.getItem('unipi.term.tabs')).toBeNull();
  });

  it('ignores corrupt storage', () => {
    localStorage.setItem('unipi.term.tabs', '{"a":{"termId":1},"b":{"termId":"t","hostId":"h"}}');
    expect(tabTermBinding('a')).toBeUndefined();
    expect(tabTermBinding('b')).toEqual({ termId: 't', hostId: 'h', cwd: '~', shell: undefined });
    localStorage.setItem('unipi.term.tabs', 'not json');
    expect(tabTermBinding('b')).toBeUndefined();
  });

  it('keeps the tail of a snapshot', () => {
    saveTermSnapshot('t1', 'x'.repeat(SNAPSHOT_MAX_CHARS + 10) + 'END');
    const snap = readTermSnapshot('t1')!;
    expect(snap.length).toBe(SNAPSHOT_MAX_CHARS);
    expect(snap.endsWith('END')).toBe(true);
    clearTermSnapshot('t1');
    expect(readTermSnapshot('t1')).toBeUndefined();
  });
});

describe('foreground process tracking (confirm before kill)', () => {
  it('a full-screen program is running', () => {
    const t = new TermProcessTracker();
    expect(t.isRunning()).toBe(false);
    t.onBufferType('alternate');
    expect(t.isRunning()).toBe(true);
    t.onBufferType('normal');
    expect(t.isRunning()).toBe(false);
  });

  it('a submitted command runs until the prompt comes back', () => {
    const t = new TermProcessTracker();
    t.onInput('sleeq\x7fp 30\r');
    expect(t.isRunning()).toBe(true);
    expect(t.hint()).toBe('sleep 30');
    t.onCursorLine('still going');
    expect(t.isRunning()).toBe(true);
    t.onCursorLine('oino ❯ ');
    expect(t.isRunning()).toBe(false);
  });

  it('ignores blank lines, comments, exit, arrows and Ctrl+C drafts', () => {
    const t = new TermProcessTracker();
    t.onInput('\r# note\rexit\r');
    expect(t.isRunning()).toBe(false);
    t.onInput('rm -rf x\x03\r');
    expect(t.isRunning()).toBe(false);
    t.onInput('\x1b[A\x1bOB\r');
    expect(t.isRunning()).toBe(false);
  });

  it('OSC 133 shell integration wins over prompt guessing', () => {
    const t = new TermProcessTracker();
    t.onOsc133('C');
    expect(t.isRunning()).toBe(true);
    t.onOsc133('D;0');
    expect(t.isRunning()).toBe(false);
  });

  it('an exited shell runs nothing', () => {
    const t = new TermProcessTracker();
    t.onBufferType('alternate');
    t.onExit();
    expect(t.isRunning()).toBe(false);
  });
});

describe('phone composer history', () => {
  it('most recent first, unique, per host', () => {
    pushTermHistory('h', 'ls');
    pushTermHistory('h', 'git status');
    expect(pushTermHistory('h', ' ls ')).toEqual(['ls', 'git status']);
    expect(readTermHistory('other')).toEqual([]);
  });
});
