import { describe, expect, it } from 'vitest';
import { escalationView, nextEscalation, queueAfter } from './queueEscalate';

describe('queue escalation state machine', () => {
  it('queued → steer → interrupt while running', () => {
    expect(nextEscalation('followUp', true)).toBe('steer');
    expect(queueAfter('followUp', 'steer')).toBe('steering');
    expect(nextEscalation('steering', true)).toBe('interrupt');
    expect(queueAfter('steering', 'interrupt')).toBeUndefined();
  });
  it('a held queue just delivers', () => {
    expect(nextEscalation('followUp', false)).toBe('deliver');
    expect(nextEscalation('steering', false)).toBe('deliver');
  });
  it('copy names the next action', () => {
    expect(escalationView('followUp', true).label).toBe('Steer now');
    expect(escalationView('steering', true).label).toBe('Send now (interrupts)');
    expect(escalationView('steering', true).tooltip).toMatch(/interrupts/);
  });
});
