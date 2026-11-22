import { describe, expect, it, vi } from 'vitest';
import { TopicClient } from './client';

const hello = { kind: 'hello' as const, capabilities: ['topics/1', 'topic:fs', 'topic:needs_you'] };
function setup(hidden = false) {
  const client = new TopicClient(() => hidden);
  const topics = vi.fn();
  const close = vi.fn();
  client.bind({ topics, close });
  client.event(hello);
  return { client, topics, hide: () => { hidden = true; client.sync(); } };
}

describe('topic visibility and compatibility', () => {
  it('watcher startup failure restores polling only for that subscription', () => {
    const { client, topics } = setup();
    const fallback = vi.fn(() => vi.fn());
    const other = vi.fn(() => vi.fn());
    client.watch('fs:/missing', vi.fn(), fallback);
    client.watch('fs:/ok', vi.fn(), other);
    client.event({ kind: 'topic', topic: 'fs:/missing', epoch: 1, seq: 0,
      snapshot: { status: 'error', error: 'watcher_start_failed' } });
    expect(fallback).toHaveBeenCalledTimes(1);
    expect(other).not.toHaveBeenCalled();
    expect(topics).toHaveBeenLastCalledWith([], ['fs:/missing']);
  });
  it('hidden panes subscribe to nothing and do not poll', () => {
    const { client, topics } = setup();
    const fallback = vi.fn(() => vi.fn());
    client.watch('fs:/tmp', vi.fn(), fallback, () => false);
    expect(topics).not.toHaveBeenCalled();
    expect(fallback).not.toHaveBeenCalled();
  });
  it('hidden document releases panes but keeps needs_you', () => {
    const { client, topics, hide } = setup();
    client.watch('fs:/tmp', vi.fn(), () => vi.fn());
    client.watch('needs_you', vi.fn(), () => vi.fn());
    hide();
    expect(topics).toHaveBeenLastCalledWith([], ['fs:/tmp']);
  });
  it('old host uses fallback and pauses it while hidden', () => {
    let hidden = false;
    const client = new TopicClient(() => hidden);
    const stop = vi.fn();
    const fallback = vi.fn(() => stop);
    client.bind({ close: vi.fn() });
    client.watch('fs:/tmp', vi.fn(), fallback);
    expect(fallback).toHaveBeenCalledTimes(1);
    hidden = true; client.sync();
    expect(stop).toHaveBeenCalledTimes(1);
    hidden = false; client.sync();
    expect(fallback).toHaveBeenCalledTimes(2);
  });
  it('unsupported topic keeps fallback even with topics capability', () => {
    const { client } = setup();
    const fallback = vi.fn(() => vi.fn());
    client.watch('agents:s', vi.fn(), fallback);
    expect(fallback).toHaveBeenCalledTimes(1);
  });
  it('refcounts subscribers, resumes cursors, snapshots on gaps, drops duplicates', () => {
    const { client, topics } = setup();
    const refresh = vi.fn();
    const a = client.watch('fs:/tmp', refresh, () => vi.fn());
    const b = client.watch('fs:/tmp', vi.fn(), () => vi.fn());
    expect(topics).toHaveBeenCalledTimes(1);
    client.event({ kind: 'topic', topic: 'fs:/tmp', epoch: 1, seq: 2, snapshot: {} });
    client.event({ kind: 'topic', topic: 'fs:/tmp', epoch: 1, seq: 3, delta: {} });
    expect(refresh).toHaveBeenCalledTimes(2);
    client.event({ kind: 'topic', topic: 'fs:/tmp', epoch: 1, seq: 3, delta: {} });
    expect(refresh).toHaveBeenCalledTimes(2);
    a(); expect(topics).toHaveBeenCalledTimes(1);
    b(); expect(topics).toHaveBeenLastCalledWith([], ['fs:/tmp']);
    client.watch('fs:/tmp', refresh, () => vi.fn());
    expect(topics).toHaveBeenLastCalledWith([{ topic: 'fs:/tmp', since: { epoch: 1, seq: 3 } }], []);
    client.event({ kind: 'topic', topic: 'fs:/tmp', epoch: 1, seq: 5, delta: {} });
    expect(topics).toHaveBeenLastCalledWith([{ topic: 'fs:/tmp' }], []);
    expect(refresh).toHaveBeenCalledTimes(2);
  });
  it('capability arrival replaces fallback, reconnect retains cursor', () => {
    const client = new TopicClient(() => false);
    const topics = vi.fn(); const stop = vi.fn();
    client.bind({ topics, close: vi.fn() });
    client.watch('fs:/tmp', vi.fn(), () => stop);
    client.event(hello);
    expect(stop).toHaveBeenCalledTimes(1);
    client.event({ kind: 'topic', topic: 'fs:/tmp', epoch: 1, seq: 9, snapshot: {} });
    client.bind();
    client.bind({ topics, close: vi.fn() });
    client.event(hello);
    expect(topics).toHaveBeenLastCalledWith([{ topic: 'fs:/tmp', since: { epoch: 1, seq: 9 } }], []);
  });
});
