import { describe, expect, it } from 'vitest';
import { runningText, runningUpdateDue } from './runningNotification';

describe('runningText (UNI-161 §5)', () => {
  it('names the activity and the elapsed time', () => {
    expect(runningText({ session: 'auth rewrite', title: 'Running', detail: 'npm test', since: 1_000, now: 186_000 })).toEqual({ title: 'auth rewrite', body: 'Running npm test · 3m 5s' });
  });
  it('works without detail or start time', () => {
    expect(runningText({ session: '', title: 'Thinking', now: 1 })).toEqual({ title: 'pi is working', body: 'Thinking' });
  });
});

describe('runningUpdateDue (UNI-223: no re-alert per chat change)', () => {
  const last = { title: 'Running', text: 's\nRunning npm test · 5s', at: 10_000 };
  it('posts the first time', () => {
    expect(runningUpdateDue(undefined, { title: 'Running', text: 'x' }, 0)).toBe(true);
  });
  it('never re-posts identical text (streaming deltas, tool count churn)', () => {
    expect(runningUpdateDue(last, { title: 'Running', text: last.text }, 999_999)).toBe(false);
  });
  it('a new activity goes out right away', () => {
    expect(runningUpdateDue(last, { title: 'Thinking', text: 's\nThinking · 5s' }, 10_001)).toBe(true);
  });
  it('the elapsed-time tick waits the gap', () => {
    expect(runningUpdateDue(last, { title: 'Running', text: 's\nRunning npm test · 6s' }, 11_000)).toBe(false);
    expect(runningUpdateDue(last, { title: 'Running', text: 's\nRunning npm test · 15s' }, 15_000)).toBe(true);
  });
});
