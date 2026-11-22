import { describe, expect, it } from 'vitest';
import { ConfigSchema, isClaimedBlockedQuestion, isLeadQuestion, isQuestion, LEAD_DEFAULTS, LeadSettingsSchema, leadState, StatusSchema, type NeedsItem } from './api';
import { countNeeds } from './needs';

describe('StatusSchema', () => {
  it('accepts partial lead progress counts', () => {
    const status = StatusSchema.parse({
      enabled: true,
      leads: [{ projectId: 'project', sessionId: 'lead', autonomy: 'high', ticketIds: ['A-1', 'A-2'], progress: { running: 2 } }],
    });

    expect(status.leads[0].progress).toEqual({ running: 2, queued: 0, done: 0 });
    const empty = StatusSchema.parse({ enabled: true, leads: [{ projectId: 'project', sessionId: 'lead', autonomy: 'high', progress: {} }] });
    expect(empty.leads[0].progress).toEqual({ running: 0, queued: 0, done: 0 });
  });

  it('still parses today\'s payload: no state, health, question, notices or lead settings', () => {
    const status = StatusSchema.parse({
      enabled: true,
      leads: [{ projectId: 'p', sessionId: 'lead', autonomy: 'high' }],
      needsYou: [{ projectId: 'p', taskId: 'P-1', status: 'blocked', question: 'EUR or USD?' }],
    });
    expect(status.leads[0]).toMatchObject({ projectId: 'p', sessionId: 'lead' });
    expect(status.leads[0]!.state).toBeUndefined();
    expect(leadState(status.leads[0]!)).toBe('working');
    expect(status.needsYou[0]!.kind).toBeUndefined();
    expect(status.notices).toEqual([]);
    expect(status.leadSettings).toEqual({ retries: 3, fallbackModel: null, warmMinutes: 30, questionTimeoutMinutes: 30 });
    expect(status.leadSettings).toEqual(LEAD_DEFAULTS);
  });

  it('reads lead state, health, question, needs-you times, notices and lead settings', () => {
    const status = StatusSchema.parse({
      enabled: true,
      leads: [
        { projectId: 'p', sessionId: 'a', autonomy: 'high', state: 'stalled', health: { kind: 'model', error: '429: quota exhausted', attempts: 8, since: 1000, nextProbeAt: 5000 }, lastActiveAt: 900 },
        { projectId: 'p', sessionId: 'b', autonomy: 'high', state: 'waiting', question: { taskId: 'P-2', excerpt: 'EUR or USD?' } },
        { projectId: 'p', sessionId: 'c', autonomy: 'high', state: 'idle', idleReason: 'unloaded' },
        { projectId: 'p', sessionId: 'd', autonomy: 'high', state: 'asleep-in-a-future-host' },
      ],
      needsYou: [{ projectId: 'p', taskId: 'P-2', status: 'in_progress', kind: 'question', question: 'EUR or USD?', askedAt: 1000, expiresAt: 2000 }],
      notices: [{ kind: 'board_created', projectId: 'p', name: 'Pee', at: 123 }],
      lead: { retries: 5, fallbackModel: ' openai/gpt-5 ', warmMinutes: 60 },
    });
    const [a, b, c, d] = status.leads;
    expect(a!.health).toEqual({ kind: 'model', error: '429: quota exhausted', attempts: 8, since: 1000, nextProbeAt: 5000 });
    expect(leadState(a!)).toBe('stalled');
    expect(b!.question).toEqual({ taskId: 'P-2', excerpt: 'EUR or USD?' });
    expect(leadState(c!)).toBe('unloaded');
    expect(leadState(d!)).toBe('working'); // an unknown future state degrades, it does not fail the status
    expect(status.needsYou[0]).toMatchObject({ kind: 'question', askedAt: 1000, expiresAt: 2000 });
    expect(status.notices).toEqual([{ kind: 'board_created', projectId: 'p', name: 'Pee', at: 123 }]);
    expect(status.leadSettings).toEqual({ retries: 5, fallbackModel: 'openai/gpt-5', warmMinutes: 60, questionTimeoutMinutes: 30 });
  });

  it('falls back to the defaults for a lead setting out of range', () => {
    const status = StatusSchema.parse({ enabled: true, lead: { retries: 99, warmMinutes: 1, questionTimeoutMinutes: 'soon', fallbackModel: 'no-slash' } });
    expect(status.leadSettings).toEqual(LEAD_DEFAULTS);
  });

  it('parses ISO string timestamps (including legacy timeoutAt) and question options from older hosts', () => {
    const askedIso = '2026-11-11T10:00:00.000Z';
    const expiresIso = '2026-11-11T10:30:00.000Z';
    const noticeIso = '2026-11-11T09:45:00.000Z';
    const status = StatusSchema.parse({
      enabled: true,
      needsYou: [
        { kind: 'question', projectId: 'p', taskId: 'P-1', status: 'in_progress', question: 'EUR or USD?', options: [['EUR', 'USD']], askedAt: askedIso, expiresAt: expiresIso },
        { kind: 'question', projectId: 'p', taskId: 'P-2', status: 'in_progress', question: 'Legacy?', askedAt: askedIso, timeoutAt: expiresIso },
      ],
      notices: [{ kind: 'board_created', projectId: 'p', name: 'Pee', at: noticeIso, text: 'Board created for Pee (p).' }],
    });
    expect(status.needsYou[0]).toMatchObject({
      askedAt: Date.parse(askedIso),
      expiresAt: Date.parse(expiresIso),
      options: [['EUR', 'USD']],
    });
    expect(status.needsYou[1]).toMatchObject({
      askedAt: Date.parse(askedIso),
      expiresAt: Date.parse(expiresIso),
    });
    expect(status.notices?.[0]?.at).toBe(Date.parse(noticeIso));
  });
});

describe('isQuestion', () => {
  const item = (over: Partial<NeedsItem>): NeedsItem => ({ projectId: 'p', taskId: 'P-1', status: 'blocked', question: 'Which one?', ...over });
  it('keeps today\'s rule for blocked tickets and distinguishes claimed blocked questions', () => {
    expect(isQuestion(item({}))).toBe(true);
    expect(isClaimedBlockedQuestion(item({}))).toBe(false);
    expect(isClaimedBlockedQuestion(item({ kind: 'question', sessionId: 'lead-1' }))).toBe(true);
    expect(isQuestion(item({ question: '   ' }))).toBe(false);
    expect(isQuestion(item({ question: null }))).toBe(false);
    expect(isLeadQuestion(item({}))).toBe(false);
  });
  it('adds a lead\'s own question: kind question while the ticket is in progress', () => {
    const asked = item({ status: 'in_progress', kind: 'question' });
    expect(isQuestion(asked)).toBe(true);
    expect(isLeadQuestion(asked)).toBe(true);
    expect(isQuestion(item({ status: 'in_progress' }))).toBe(false); // an in-progress ticket without kind is not a question
    expect(isQuestion(item({ status: 'in_progress', kind: 'approval' }))).toBe(false);
    expect(isQuestion(item({ status: 'in_progress', kind: 'question', question: '' }))).toBe(false);
  });
  it('never takes an approval for a question', () => {
    const approval = item({ status: 'in_progress', kind: 'question', workerSessionId: 'w', requestId: 'r', sessionId: 's', tool: 'bash', ceiling: 'medium', reason: 'needs High' });
    expect(isQuestion(approval)).toBe(false);
  });
  it('counts a lead question as something waiting on the owner', () => {
    expect(countNeeds([item({ status: 'in_progress', kind: 'question' }), item({}), item({ status: 'in_progress' })])).toBe(2);
  });
});

describe('ConfigSchema', () => {
  it('leaves `lead` out when it is not sent, so a toggle never resets the host\'s lead settings', () => {
    expect(ConfigSchema.parse({ enabled: true })).toEqual({ enabled: true });
    expect(ConfigSchema.parse({ enabled: true, pausedGraphPolicy: 'reuse' })).toEqual({ enabled: true, pausedGraphPolicy: 'reuse' });
  });
  it('fills the defaults of a partial lead object and turns an empty fallback into null', () => {
    expect(ConfigSchema.parse({ enabled: true, lead: { retries: 4, fallbackModel: '  ' } }).lead).toEqual({ retries: 4, fallbackModel: null, warmMinutes: 30, questionTimeoutMinutes: 30 });
  });
  it('rejects values outside the spec ranges with an inline message', () => {
    const message = (lead: object) => LeadSettingsSchema.safeParse(lead).error?.issues[0]?.message;
    expect(message({ retries: 11 })).toBe('0 to 10.');
    expect(message({ retries: -1 })).toBe('0 to 10.');
    expect(message({ warmMinutes: 4 })).toBe('5 to 1440.');
    expect(message({ warmMinutes: 1441 })).toBe('5 to 1440.');
    expect(message({ questionTimeoutMinutes: 0 })).toBe('1 to 1440.');
    expect(message({ retries: 1.5 })).toBe('Use a whole number.');
    expect(message({ fallbackModel: 'gpt-5' })).toBe('Use provider/model, or leave empty.');
    expect(LeadSettingsSchema.safeParse({ retries: 0, warmMinutes: 5, questionTimeoutMinutes: 1440, fallbackModel: 'openai/gpt-5' }).success).toBe(true);
  });
});
