/**
 * [chat-events] describeEvent goldens: every kind of the corpus answers its
 * own kind id, icon, "Source · title", meta, tone and body format; hidden
 * kinds answer `null`; nothing renders raw JSON or XML as prose.
 */
import { describe, expect, it } from 'vitest';
import { CHAT_EVENT_CORPUS } from '../../dev/mock/fixtures/chatEvents';
import type { SessionEntry } from '../session/types';
import { buildTurns } from '../session/turns';
import { describeEvent, eventAudience, eventVisible, firstLine, xmlTag, AUDIENCE_LABEL, AUDIENCE_TITLE } from './eventKinds';

describe('the audience badge (owner 2026-11-06)', () => {
  it('names the audiences the way the user reads them', () => {
    expect(AUDIENCE_LABEL.user).toBe('Visible to you only');
    expect(AUDIENCE_LABEL.agent).toBe('Hidden from you');
    expect(AUDIENCE_LABEL.both).toBe('You & agent');
    for (const title of Object.values(AUDIENCE_TITLE)) expect(title.length).toBeGreaterThan(10);
  });
});

describe('describeEvent (chat event corpus)', () => {
  it.each(CHAT_EVENT_CORPUS.map(([kind, e]) => [kind ?? 'hidden', e.customType ?? e.message?.role ?? e.type, e] as const))(
    '%s ← %s',
    (kind, _type, entry) => {
      const v = describeEvent(entry);
      if (kind === 'hidden') {
        expect(v ?? null).toBeNull();
        expect(eventVisible(entry)).toBe(false);
        return;
      }
      expect(v).toBeTruthy();
      expect(v!.kind).toBe(kind);
      expect(eventVisible(entry)).toBe(true);
      expect(v!.title.trim()).not.toBe('');
      expect(v!.title).not.toMatch(/[<{]/); // never XML / JSON in the one-liner
      if (v!.body?.format === 'markdown') expect(v!.body.text).not.toMatch(/^\s*[{<]/);
      expect({ kind: v!.kind, icon: v!.icon, source: v!.source, title: v!.title, meta: v!.meta, tone: v!.tone, body: v!.body?.format, renderer: v!.renderer }).toMatchSnapshot();
    },
  );

  it('memory recall reads "recalled N · first title" and the project', () => {
    const v = describeEvent(CHAT_EVENT_CORPUS[0]![1])!;
    expect(`${v.source} · ${v.title}`).toBe('Memory · recalled 5 · oino_handoff_20261102_finalize_graph');
    expect(v.meta).toBe('oino');
    expect(describeEvent(CHAT_EVENT_CORPUS[1]![1])!.title).toBe('no memories yet for openmontage');
  });

  it('memory save card reads "saved <title> +N"', () => {
    const v = describeEvent(CHAT_EVENT_CORPUS.find(([k]) => k === 'memory-saved')![1])!;
    expect(v.title).toBe('saved oino_phone_sheet_scrim_above_sheet_zindex_specificity +1');
    expect(v.tone).toBe('ok');
  });

  it('task notifications become Subagent rows with the <result> as markdown', () => {
    const v = describeEvent(CHAT_EVENT_CORPUS.find(([k]) => k === 'subagent-notification')![1])!;
    expect(v.title).toBe('Trace film sessions · done');
    expect(v.meta).toBe('3m 40s · 489k tokens');
    expect(v.body).toEqual({ format: 'markdown', text: expect.stringContaining('Key findings') });
    // the same XML inside a legacy unipi-response
    const legacy = { ...CHAT_EVENT_CORPUS.find(([k]) => k === 'subagent-notification')![1], customType: 'unipi-response', display: false } as SessionEntry;
    expect(describeEvent(legacy)!.kind).toBe('subagent-notification');
    expect(eventVisible(legacy)).toBe(true);
  });

  it('user !bash rows read command / exit / output (they were blank)', () => {
    const ok = describeEvent(CHAT_EVENT_CORPUS.find(([, e]) => e.id === 'ev28')![1])!;
    expect([ok.title, ok.meta, ok.tone]).toEqual(['git status --short', 'exit 0', undefined]);
    expect(ok.body!.text).toContain('chat-events-parity.md');
    const bad = describeEvent(CHAT_EVENT_CORPUS.find(([, e]) => e.id === 'ev29')![1])!;
    expect([bad.meta, bad.tone]).toEqual(['exit 101', 'error']);
  });

  it('an unknown message with only details shows them as JSON in a code block, never as prose', () => {
    const v = describeEvent(CHAT_EVENT_CORPUS.find(([k]) => k === 'custom')![1])!;
    expect(v.body!.format).toBe('json');
    expect(v.source).toBe('Acme');
    expect(v.renderer).toBe('customMessage');
  });

  it('an unknown display:false message stays hidden; an unknown custom state entry is left to its renderer', () => {
    expect(eventVisible({ type: 'custom_message', id: 'u1', customType: 'acme:secret', content: 'x', display: false } as SessionEntry)).toBe(false);
    expect(describeEvent({ type: 'custom', id: 'u2', customType: 'acme:state', data: { a: 1 } } as SessionEntry)).toBeUndefined();
  });

  it('bodies are capped (no unbounded DOM)', () => {
    const v = describeEvent({ type: 'custom_message', id: 'big', customType: 'unipi-watchdog', content: 'x'.repeat(200_000), display: true } as SessionEntry)!;
    expect(v.body!.text.length).toBeLessThanOrEqual(60_002);
  });

  it('helpers', () => {
    expect(firstLine('## 🧠 Memory System Active\nx')).toBe('Memory System Active');
    expect(firstLine('<task-notification>\n<x>')).toBe('');
    expect(xmlTag('<a>1 &amp; 2</a>', 'a')).toBe('1 & 2');
  });
});

describe('turns: where event cards render', () => {
  const user = (id: string, text: string): SessionEntry => ({ type: 'message', id, timestamp: '2026-11-03T10:00:00Z', message: { role: 'user', content: text, timestamp: 1 } }) as SessionEntry;
  const assistant = (id: string, parts: unknown[], stop = 'stop'): SessionEntry =>
    ({ type: 'message', id, timestamp: '2026-11-03T10:00:01Z', message: { role: 'assistant', content: parts, stopReason: stop, timestamp: 2 } }) as SessionEntry;
  const corpus = (id: string) => CHAT_EVENT_CORPUS.find(([, e]) => e.id === id)![1];

  it('a recall before the work sits above the fold, a save card after the answer below it, a mid-run notification stays in the fold', () => {
    const entries = [
      user('u1', 'fix it'),
      corpus('ev02'), // display:false recall → still shown
      assistant('a1', [{ type: 'toolCall', id: 't1', name: 'bash', arguments: { command: 'ls' } }], 'toolUse'),
      { type: 'message', id: 'r1', message: { role: 'toolResult', toolCallId: 't1', toolName: 'bash', content: [{ type: 'text', text: 'ok' }], timestamp: 3 } } as SessionEntry,
      corpus('ev10'), // bg notification mid-run
      assistant('a2', [{ type: 'text', text: 'Done.' }]),
      corpus('ev05'), // memory save after the answer
      corpus('ev33'), // empty continue → hidden
    ];
    const turns = buildTurns({ entries, leafId: 'ev33', entriesRev: 1, liveTools: {}, run: { running: false } });
    const seg = turns[0]!.segments[0]!;
    expect(seg.before?.map((n) => n.key)).toEqual(['ev02']);
    expect(seg.after?.map((n) => n.key)).toEqual(['ev05']);
    expect(seg.work.map((w) => w.key)).toContain('ev10');
    expect(seg.answer?.text).toBe('Done.');
    expect(JSON.stringify(seg)).not.toContain('ev33');
  });

  it('discriminates user-only, agent-only and shared (both) entries, and lifts trailing compaction out of the fold', () => {
    // display:false recall reminder → agent only
    expect(describeEvent(corpus('ev02'))!.audience).toBe('agent');
    // custom state card (memory-saved) → user only
    expect(describeEvent(corpus('ev05'))!.audience).toBe('user');
    // !bash without excludeFromContext → both; with excludeFromContext → user only
    expect(describeEvent(corpus('ev28'))!.audience).toBe('both');
    expect(describeEvent({ ...corpus('ev28'), message: { ...corpus('ev28').message!, excludeFromContext: true } })!.audience).toBe('user');
    // system prompt checkpoint → agent only
    expect(eventAudience({ type: 'message', id: 'sys1', message: { role: 'system', content: 'You are powered by Fusion.' } } as SessionEntry)).toBe('agent');
    // compaction summary → both
    const cmp = { type: 'compaction', id: 'cmp1', summary: '## Summary\nKept full history.', tokensBefore: 80_000 } as unknown as SessionEntry;
    expect(eventAudience(cmp)).toBe('both');

    const entries = [
      user('u1', 'summarize'),
      assistant('a1', [{ type: 'text', text: 'Here is the answer.' }]),
      cmp,
      corpus('ev12'), // unipi-compaction card
    ];
    const turns = buildTurns({ entries, leafId: 'ev12', entriesRev: 1, liveTools: {}, run: { running: false } });
    expect(turns[0]!.segments[0]!.answer?.text).toBe('Here is the answer.');
    // Between-turn compaction becomes its own top-level compaction turn unit (never folded away).
    expect(turns[1]!.kind).toBe('compaction');
    expect(turns[1]!.compaction).toBe(cmp);
  });
});
