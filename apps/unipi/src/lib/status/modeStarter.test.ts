/**
 * Working-mode starters (WP-A14 acceptance): the chip's starters and the
 * Ctrl/Cmd+Shift+M cycle rewrite only the leading command, keep the typed
 * words, and end at plain.
 */
import { describe, expect, it } from 'vitest';
import { cycleStarter, starterOf, stripStarter, withStarter } from './modeStarter';

describe('starterOf / stripStarter', () => {
  it('recognises the leading command of each mode', () => {
    expect(starterOf('/goal ship it')).toBe('goal');
    expect(starterOf('/goal')).toBe('goal');
    expect(starterOf('/ralph start tidy')).toBe('ralph');
    expect(starterOf('/graph map the repo')).toBe('graph');
    expect(starterOf('ship it')).toBe('none');
    expect(starterOf('/regular')).toBe('none');
    expect(starterOf('please /goal inside')).toBe('none');
  });
  it('keeps the words after the command', () => {
    expect(stripStarter('/goal ship it')).toBe('ship it');
    expect(stripStarter('  /ralph start tidy  ')).toBe('tidy');
    expect(stripStarter('ship it')).toBe('ship it');
  });
});

describe('withStarter', () => {
  it('writes the command in front of the same words (idempotent)', () => {
    expect(withStarter('ship it', 'goal')).toBe('/goal ship it');
    expect(withStarter('/graph ship it', 'goal')).toBe('/goal ship it');
    expect(withStarter('/goal ship it', 'none')).toBe('ship it');
  });
  it('never drops a draft that has no command yet', () => {
    expect(withStarter('write the tests first', 'ralph')).toBe('/ralph start write the tests first');
  });
});

describe('cycleStarter (Ctrl/Cmd+Shift+M)', () => {
  it('goes Goal → Ralph → Graph → plain → Goal', () => {
    let step = cycleStarter('ship it');
    expect(step).toEqual({ text: '/goal ship it', mode: 'goal' });
    step = cycleStarter(step.text);
    expect(step.mode).toBe('ralph');
    expect(step.text).toBe('/ralph start ship it');
    step = cycleStarter(step.text);
    expect(step.mode).toBe('graph');
    expect(step.text).toBe('/graph ship it');
    step = cycleStarter(step.text);
    expect(step).toEqual({ text: 'ship it', mode: 'none' });
    expect(cycleStarter(step.text).mode).toBe('goal');
  });
});
