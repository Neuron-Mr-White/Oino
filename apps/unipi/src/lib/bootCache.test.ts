import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { BOOT_CARDS_MAX, BOOT_WRITE_MS, readBootCards, readBootHosts, writeBootCards, writeBootHosts } from './bootCache';
import type { SessionCard } from './host/client';

const card = (id: string, updated_at: number): SessionCard => ({ session_id: id, run_state: 'idle', cwd: '/p', updated_at });
const host = { id: 'h1', name: 'This computer', ticket: 'endpoint-ticket-secret', paired_at: 0, last_path: 'local' as const, transport: 'local' as const };

beforeEach(() => {
  localStorage.clear();
  vi.useFakeTimers();
});
afterEach(() => vi.useRealTimers());

describe('boot cache', () => {
  it('reads nothing on a first launch or from bad data', () => {
    expect(readBootHosts()).toBeUndefined();
    expect(readBootCards('h1')).toBeUndefined();
    localStorage.setItem('unipi.boot.hosts.v1', '{nope');
    localStorage.setItem('unipi.boot.cards.v1.h1', '"x"');
    expect(readBootHosts()).toBeUndefined();
    expect(readBootCards('h1')).toBeUndefined();
  });

  it('writes the hosts once after a quiet second, without their pairing tickets', () => {
    writeBootHosts(() => [host]);
    writeBootHosts(() => [host]);
    expect(localStorage.getItem('unipi.boot.hosts.v1')).toBeNull();
    vi.advanceTimersByTime(BOOT_WRITE_MS);
    expect(localStorage.getItem('unipi.boot.hosts.v1')).not.toContain('secret');
    expect(readBootHosts()).toEqual([{ ...host, ticket: '-' }]);
  });

  it('writes the latest cards, newest first, capped', () => {
    let list = [card('old', 1)];
    writeBootCards('h1', () => list);
    list = Array.from({ length: BOOT_CARDS_MAX + 20 }, (_, i) => card(`s${i}`, i));
    vi.advanceTimersByTime(BOOT_WRITE_MS);
    const read = readBootCards('h1')!;
    expect(read).toHaveLength(BOOT_CARDS_MAX);
    expect(read[0]!.session_id).toBe(`s${BOOT_CARDS_MAX + 19}`);
    expect(readBootCards('h2')).toBeUndefined();
  });
});
