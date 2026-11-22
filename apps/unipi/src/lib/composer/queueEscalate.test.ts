import { describe, expect, it } from 'vitest';
import { escalationView, kindView, nextEscalation, queueAfter } from './queueEscalate';

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
    expect(escalationView('followUp', true).label).toBe('Send at next step');
    expect(escalationView('steering', true).label).toBe('Stop the current step and send now');
    expect(escalationView('steering', true).tooltip).toMatch(/^Stop the current step and send now/);
    expect(escalationView('steering', false).label).toBe('Send now');
  });
  it('row chips use plain words', () => {
    expect(kindView('steering')).toEqual({ label: 'Next step', tooltip: 'Sent to the agent after its current step' });
    expect(kindView('followUp')).toEqual({ label: 'After turn', tooltip: 'Sent when the agent finishes this turn' });
  });
});
