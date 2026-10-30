// @vitest-environment jsdom
/** sendMode (WP-A11, Z-52): global default + modifier matrix. */
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import { enterMode, followUpMode, modeLabel, oppositeMode, outboxMode, sendModeForEnter, setFollowUpMode } from './sendMode';

const plain = { metaKey: false, ctrlKey: false };
const cmd = { metaKey: true, ctrlKey: false };
const ctrl = { metaKey: false, ctrlKey: true };

beforeEach(() => localStorage.clear());
afterEach(() => localStorage.clear());

describe('follow-up mode setting', () => {
  it('defaults to queue', () => {
    expect(followUpMode()).toBe('queue');
  });

  it('persists across a reload', () => {
    setFollowUpMode('steer');
    expect(localStorage.getItem('unipi:composer:followUpMode')).toBe('steer');
    expect(followUpMode()).toBe('steer');
  });

  it('oppositeMode flips once', () => {
    expect(oppositeMode('queue')).toBe('steer');
    expect(oppositeMode('steer')).toBe('queue');
    expect(modeLabel('queue')).toBe('Queue');
    expect(modeLabel('steer')).toBe('Steer');
  });
});

describe('Enter / modifier matrix (running)', () => {
  it('Enter does the default; Cmd/Ctrl+Enter the opposite', () => {
    setFollowUpMode('queue');
    expect(enterMode(plain)).toBe('queue');
    expect(enterMode(cmd)).toBe('steer');
    expect(enterMode(ctrl)).toBe('steer');
    setFollowUpMode('steer');
    expect(enterMode(plain)).toBe('steer');
    expect(enterMode(cmd)).toBe('queue');
    expect(enterMode(ctrl)).toBe('queue');
  });

  it('maps to session command modes while running', () => {
    setFollowUpMode('queue');
    expect(sendModeForEnter(plain, true)).toBe('followUp');
    expect(sendModeForEnter(cmd, true)).toBe('steer');
    setFollowUpMode('steer');
    expect(sendModeForEnter(plain, true)).toBe('steer');
    expect(sendModeForEnter(ctrl, true)).toBe('followUp');
  });

  it('while idle every Enter sends a prompt', () => {
    setFollowUpMode('queue');
    expect(sendModeForEnter(plain, false)).toBe('prompt');
    expect(sendModeForEnter(cmd, false)).toBe('prompt');
    setFollowUpMode('steer');
    expect(sendModeForEnter(plain, false)).toBe('prompt');
    expect(outboxMode('steer', false)).toBe('prompt');
    expect(outboxMode('queue', true)).toBe('followUp');
  });
});
