// @vitest-environment jsdom
import { beforeEach, describe, expect, it } from 'vitest';
import { patchWorkbenchView, readDraft, readWorkbenchView, resetWorkbenchState, resolveTicket, writeDraft } from './workbenchState';

const t = (project: string, task: string) => ({ project, task });
beforeEach(() => resetWorkbenchState());

describe('workbenchState', () => {
  it('keeps the view per host and reads it back from sessionStorage after the memory copy is gone', () => {
    patchWorkbenchView('a', { open: 'review', project: 'oino', search: 'fix', task: t('oino', 'OIN-2'), queue: [t('oino', 'OIN-1'), t('oino', 'OIN-2')], listScroll: 120, ticketScroll: { 'OIN-2': 340 } });
    expect(readWorkbenchView('b')).toEqual({});
    const stored = sessionStorage.getItem('unipi:superagent:workbench:a')!;
    resetWorkbenchState();
    sessionStorage.setItem('unipi:superagent:workbench:a', stored);
    expect(readWorkbenchView('a')).toMatchObject({ open: 'review', project: 'oino', search: 'fix', task: t('oino', 'OIN-2'), listScroll: 120, ticketScroll: { 'OIN-2': 340 } });
  });

  it('an undefined field is removed, and a corrupt entry reads as empty', () => {
    patchWorkbenchView('a', { open: 'review', project: 'oino' });
    patchWorkbenchView('a', { open: undefined });
    expect(readWorkbenchView('a')).toEqual({ project: 'oino' });
    resetWorkbenchState();
    sessionStorage.setItem('unipi:superagent:workbench:a', '{"open":"nope","task":5}');
    expect(readWorkbenchView('a')).toEqual({});
  });

  it('a draft survives, and blank text (what sending leaves) removes it', () => {
    writeDraft('a', 'oino/OIN-1', 'hello');
    writeDraft('a', 'answer:oino/OIN-2', 'yes');
    resetWorkbenchState();
    expect(readDraft('a', 'oino/OIN-1')).toBe('');
    writeDraft('a', 'oino/OIN-1', 'hello');
    const raw = sessionStorage.getItem('unipi:superagent:workbench:drafts:a')!;
    expect(JSON.parse(raw)).toEqual({ 'oino/OIN-1': 'hello' });
    writeDraft('a', 'oino/OIN-1', '');
    expect(readDraft('a', 'oino/OIN-1')).toBe('');
    expect(sessionStorage.getItem('unipi:superagent:workbench:drafts:a')).toBeNull();
  });

  it('resolveTicket: the same ticket, else the next one still listed, else the one before, else none', () => {
    const queue = [t('p', '1'), t('p', '2'), t('p', '3'), t('p', '4')];
    expect(resolveTicket(t('p', '2'), queue, queue)).toEqual(t('p', '2'));
    expect(resolveTicket(t('p', '2'), queue, [t('p', '1'), t('p', '3'), t('p', '4')])).toEqual(t('p', '3'));
    expect(resolveTicket(t('p', '2'), queue, [t('p', '1'), t('p', '4')])).toEqual(t('p', '4'));
    expect(resolveTicket(t('p', '4'), queue, [t('p', '1'), t('p', '3')])).toEqual(t('p', '3'));
    expect(resolveTicket(t('p', '2'), queue, [])).toBeUndefined();
    expect(resolveTicket(undefined, queue, queue)).toBeUndefined();
  });
});
