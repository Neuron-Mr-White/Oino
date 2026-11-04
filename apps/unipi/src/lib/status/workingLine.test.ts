import { describe, expect, it } from 'vitest';
import type { RunInfo } from '../session/types';
import { workingHead, workingLineOf } from './workingLine';

const idle: RunInfo = { phase: 'idle', running: false };

describe('workingLineOf', () => {
  it('is nothing while idle', () => {
    expect(workingLineOf({ run: idle })).toBeUndefined();
  });
  it('working while the agent streams, with the extension message when set', () => {
    const run: RunInfo = { phase: 'running', running: true, startedAt: 5 };
    expect(workingLineOf({ run })).toMatchObject({ kind: 'working', text: 'Working…', since: 5 });
    expect(workingLineOf({ run, message: 'Subagent running · Ctrl+B to run in background' })?.text).toBe('Subagent running');
  });
  it('names what an idle-but-Working session waits on', () => {
    const run: RunInfo = { phase: 'waiting', running: false, waitingOn: ['1 subagent'] };
    expect(workingLineOf({ run })).toMatchObject({ kind: 'working', text: 'Waiting on 1 subagent', openRunning: true });
  });
  it('compaction and retry read as such', () => {
    expect(workingLineOf({ run: { phase: 'compacting', running: false } })?.text).toBe('Compacting…');
    expect(workingLineOf({ run: { phase: 'retrying', running: false } })?.text).toBe('Retrying…');
  });
  it('a waiting ask_user question wins, still (no clock)', () => {
    const line = workingLineOf({ run: { phase: 'running', running: true }, ask: 'Which DB?' });
    expect(line).toMatchObject({ kind: 'needs-you', text: 'Which DB?' });
    expect(line?.since).toBeUndefined();
  });
  it('background work without a wake is the quiet kind; the tray summary alone too', () => {
    expect(workingLineOf({ run: { ...idle, backgroundOn: ['background task t1'] } })).toMatchObject({ kind: 'background', openRunning: true });
    expect(workingLineOf({ run: idle, tray: 'Subagents 1 running' })).toMatchObject({ kind: 'background', text: 'Subagents 1 running' });
  });
  it('drops key hints from a working message', () => {
    expect(workingHead('Ctrl+B to run in background')).toBeUndefined();
    expect(workingHead('Thinking…')).toBe('Thinking…');
  });
});
