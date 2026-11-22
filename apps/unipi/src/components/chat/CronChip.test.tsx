/** [ui-chat-bits] The cron chat header chip's pure parts. */
import { describe, expect, it } from 'vitest';
import { cronJobHref, cronJobOf } from './CronChip';

describe('cronJobOf', () => {
  it('only a cron-run card has a job', () => {
    expect(cronJobOf(undefined)).toBeUndefined();
    expect(cronJobOf({})).toBeUndefined();
    expect(cronJobOf({ source: 'user', cron_job_id: 'x' })).toBeUndefined();
  });
  it('names the job from the card, else the fallback lookup, else its id', () => {
    expect(cronJobOf({ source: 'cron', cron_job_id: 'digest', cron_job_name: 'Daily digest' })).toEqual({ id: 'digest', name: 'Daily digest' });
    expect(cronJobOf({ source: 'cron', cron_job_id: 'digest' }, 'From runs')).toEqual({ id: 'digest', name: 'From runs' });
    expect(cronJobOf({ source: 'cron', cron_job_id: 'digest' })).toEqual({ id: 'digest', name: 'digest' });
    expect(cronJobOf({ source: 'cron' })).toEqual({ id: undefined, name: undefined });
  });
});

describe('cronJobHref', () => {
  it('deep-links the job on its PC', () => {
    expect(cronJobHref('host 1', 'a/b')).toBe('/cron?job=a%2Fb&host=host%201');
    expect(cronJobHref('h')).toBe('/cron?host=h');
  });
});
