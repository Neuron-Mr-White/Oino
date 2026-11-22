import { describe, expect, it, vi } from 'vitest';
import { fetchFullEntries, fetchFullEntriesSettled } from './fullEntries';

describe('full entry chunks', () => {
  it('assembles escaped Unicode JSON without losing fields', async () => {
    const entry = { id: 'e', type: 'message', message: { role: 'assistant', details: ['中文\n"', 42] } };
    const text = JSON.stringify(entry);
    const cmd = vi.fn(async (command) => command.chunkOffset === undefined
      ? { entries: [{ id: 'e', truncated: true }] }
      : { json: text.slice(command.chunkOffset, command.chunkOffset + 7), offset: command.chunkOffset,
          nextOffset: command.chunkOffset + 7 < text.length ? command.chunkOffset + 7 : null });
    expect(await fetchFullEntries(cmd as never, ['e'])).toEqual([entry]);
    expect(cmd.mock.calls.length).toBeGreaterThan(2);
  });

  it('rejects a nonadvancing chunk cursor', async () => {
    const cmd = vi.fn(async (command) => command.chunkOffset === undefined
      ? { entries: [{ id: 'e', truncated: true }] }
      : { json: 'x', offset: 0, nextOffset: 0 });
    await expect(fetchFullEntries(cmd as never, ['e'])).rejects.toThrow('did not advance');
  });

  it('does not refetch whole entries and retains legacy bounded replies', async () => {
    const whole = { id: 'a', type: 'message' };
    const cut = { id: 'b', truncated: true };
    const cmd = vi.fn(async () => ({ entries: [whole, cut] }));
    expect(await fetchFullEntries(cmd as never, ['a', 'b'])).toEqual([whole, cut]);
    expect(cmd).toHaveBeenCalledTimes(2);
  });
});

describe('w7-ui G3-1: per-id settling', () => {
  const chunked = (entries: Record<string, unknown>, failing: string[] = []) => vi.fn(async (command: { ids?: string[]; chunkOffset?: number }) => {
    const id = command.ids![0]!;
    if (command.chunkOffset === undefined) return { entries: command.ids!.map((i) => ({ id: i, truncated: true })) };
    if (failing.includes(id)) throw { code: 'E_INTERNAL', message: `boom ${id}` };
    const text = JSON.stringify(entries[id]);
    const end = Math.min(command.chunkOffset + 9, text.length);
    return { json: text.slice(command.chunkOffset, end), offset: command.chunkOffset, nextOffset: end < text.length ? end : null };
  });
  const call = { id: 'a', type: 'message', message: { role: 'assistant', content: [{ type: 'text', text: 'x' }] } };
  const answer = { id: 'b', type: 'message', message: { role: 'assistant', content: [{ type: 'text', text: 'y' }] } };

  it('one failing id does not fail the batch: the others come back, the failure names its reason', async () => {
    const cmd = chunked({ a: call, b: answer }, ['a']);
    const result = await fetchFullEntriesSettled(cmd as never, ['a', 'b']);
    expect(result.entries).toEqual([answer]);
    expect(result.errors).toEqual({ a: 'boom a' });
    expect(await fetchFullEntries(cmd as never, ['a', 'b'])).toEqual([answer]);
  });

  it('fetchFullEntries rejects only when no id could be fetched', async () => {
    await expect(fetchFullEntries(chunked({ a: call }, ['a']) as never, ['a'])).rejects.toThrow('boom a');
  });

  it('a lost preview call falls back to the chunk path instead of failing every id', async () => {
    const inner = chunked({ a: call });
    const cmd = vi.fn(async (command: { ids?: string[]; chunkOffset?: number }) => {
      if (command.chunkOffset === undefined) throw new Error('offline');
      return inner(command);
    });
    expect((await fetchFullEntriesSettled(cmd as never, ['a'])).entries).toEqual([call]);
  });

  it('repairs a message the host sent without its role; an unrecognisable one is an error', async () => {
    const strip = (e: { message: Record<string, unknown> }) => ({ ...e, message: Object.fromEntries(Object.entries(e.message).filter(([k]) => k !== 'role')) });
    const result = await fetchFullEntriesSettled(
      chunked({
        a: strip({ ...call, message: { ...call.message, provider: 'p', model: 'm', stopReason: 'stop' } }),
        b: { id: 'b', type: 'message', message: { toolCallId: 't', toolName: 'bash', content: [], isError: false } },
        c: { id: 'c', type: 'message', message: { content: [{ type: 'text', text: 'hi' }] } },
        d: { id: 'd', type: 'message', message: { weird: true } },
      }) as never,
      ['a', 'b', 'c', 'd'],
    );
    expect(result.entries.map((e) => [e.id, e.message?.role])).toEqual([['a', 'assistant'], ['b', 'toolResult'], ['c', 'user']]);
    expect(result.errors.d).toBe('Entry has no role');
  });
});
