// @vitest-environment jsdom
/** Phone shell helpers (phone spec §1, §2.1, §3.3). */
import { cleanup, fireEvent, render, screen, waitFor } from '@solidjs/testing-library';
import { afterEach, describe, expect, it } from 'vitest';
import { tabTarget } from '../shell/TabBar';
import { phoneScale } from '../shell/PhoneShell';
import { phoneSize } from '../shell/AppShell';
import { matchesQuery, projectName, shortTime } from './ChatsList';
import { ActionSheet, confirmSheet } from './Sheet';

afterEach(() => cleanup());

describe('tab bar (§1.1)', () => {
  it('another tab restores its last location, else its root', () => {
    expect(tabTarget('settings', '/chat', '/settings/providers')).toBe('/settings/providers');
    expect(tabTarget('cron', '/chat', undefined)).toBe('/cron');
  });
  it('the active tab pops to root, or scrolls to top at the root', () => {
    expect(tabTarget('settings', '/settings/providers', '/settings/providers')).toBe('/settings');
    expect(tabTarget('chat', '/chat', undefined)).toBe('top');
    expect(tabTarget('files', '/files?path=%2Fsrv', undefined)).toBe('/files');
  });
});

describe('size classes (§2.1, §2.2)', () => {
  it('compact / medium / short', () => {
    expect(phoneSize(411, 914)).toEqual({ size: 'compact', short: false });
    expect(phoneSize(720, 1000)).toEqual({ size: 'medium', short: false });
    expect(phoneSize(850, 390)).toEqual({ size: 'medium', short: true });
  });
  it('UI font size → --ph-scale', () => {
    expect(phoneScale(15)).toBe(1);
    expect(phoneScale(18)).toBe(1.2);
    expect(phoneScale(12)).toBe(0.85);
    expect(phoneScale(undefined)).toBe(1);
  });
});

describe('Chats rows (§3.3)', () => {
  const now = new Date('2026-11-04T15:00:00').getTime();
  it('short captions', () => {
    expect(shortTime(now - 20_000, now)).toBe('now');
    expect(shortTime(now - 5 * 60_000, now)).toBe('5m');
    expect(shortTime(now - 3 * 3_600_000, now)).toMatch(/^\d{2}:\d{2}$/);
    expect(shortTime(undefined, now)).toBe('');
  });
  it('project basename, never a path; search over title + project', () => {
    expect(projectName('/home/me/projects/app/')).toBe('app');
    expect(projectName(undefined)).toBeUndefined();
    expect(matchesQuery('Fix tests', 'unipi', 'UNI')).toBe(true);
    expect(matchesQuery('Fix tests', 'unipi', 'docs')).toBe(false);
  });
});

describe('ActionSheet + confirmSheet (§2.8)', () => {
  it('a confirm row asks in a second sheet before running', async () => {
    let ran = 0;
    let closed = 0;
    render(() => (
      <ActionSheet open onClose={() => closed++} title="Chat" sections={[[{ id: 'del', label: 'Delete', tone: 'danger', confirm: 'Delete it?', onSelect: () => ran++ }]]} />
    ));
    fireEvent.click(screen.getByTestId('action-del'));
    expect(closed).toBe(1);
    await waitFor(() => expect(screen.getByTestId('confirm-ok')).toBeTruthy());
    expect(ran).toBe(0);
    fireEvent.click(screen.getByTestId('confirm-ok'));
    await waitFor(() => expect(ran).toBe(1));
  });
  it('confirmSheet resolves false on Cancel', async () => {
    const answer = confirmSheet({ title: 'Discard changes?', confirm: 'Discard', tone: 'danger' });
    await waitFor(() => expect(screen.getByTestId('confirm-cancel')).toBeTruthy());
    fireEvent.click(screen.getByTestId('confirm-cancel'));
    await expect(answer).resolves.toBe(false);
  });
});
