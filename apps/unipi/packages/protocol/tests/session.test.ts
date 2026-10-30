import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import {
  EventsControlSchema,
  HubEventSchema,
  PROTO_VERSION,
  SessionArchiveRequestSchema,
  SessionCloseRequestSchema,
  SessionCmdControlSchema,
  SessionMetaSetRequestSchema,
  SessionForkRequestSchema,
  SessionNewRequestSchema,
  SessionOpenRequestSchema,
  SessionRenameRequestSchema,
  SessionReplySchema,
  SessionSnapshotSchema,
  SessionsListDataSchema,
  SessionsListRequestSchema,
  EventsOpenRequestSchema,
  hostEventBanner,
} from '../src/index.js';

type Fixture = { name: string; schema: string; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(readFileSync(new URL('../../../protocol/fixtures/session.json', import.meta.url), 'utf8'));

/** The two containers a session stream speaks (host.md §7.3): control frames and requests. */
const SessionControlSchema = z.union([EventsControlSchema, SessionCmdControlSchema]);
const SessionRequestSchema = z.union([
  SessionsListRequestSchema,
  EventsOpenRequestSchema,
  SessionNewRequestSchema,
  SessionOpenRequestSchema,
  SessionForkRequestSchema,
  SessionArchiveRequestSchema,
  SessionRenameRequestSchema,
  SessionCloseRequestSchema,
  SessionMetaSetRequestSchema,
]);

const schemas: Record<string, z.ZodTypeAny> = {
  SessionControl: SessionControlSchema,
  SessionRequest: SessionRequestSchema,
  SessionsListData: SessionsListDataSchema,
  SessionSnapshot: SessionSnapshotSchema,
  SessionReply: SessionReplySchema,
  HubEvent: HubEventSchema,
};

describe('session contract fixtures (v2 host.md §7.3–7.4)', () => {
  it('proto is 2.0.0 (session layer)', () => {
    expect(PROTO_VERSION.startsWith('2.')).toBe(true);
  });

  it('covers every schema', () => {
    for (const name of Object.keys(schemas)) {
      expect(fixtures.some((fixture) => fixture.schema === name), name).toBe(true);
    }
  });

  for (const fixture of fixtures) {
    it(fixture.name, () => {
      const schema = schemas[fixture.schema];
      expect(schema, fixture.schema).toBeDefined();
      expect(schema!.safeParse(fixture.value).success).toBe(fixture.ok);
    });
  }
});

describe('session host events', () => {
  it('maps the host-only kinds to banner text', () => {
    expect(hostEventBanner({ kind: 'crashed', error: 'tool body exploded' })).toContain('tool body exploded');
    expect(hostEventBanner({ type: 'crashed', error: 'tool body exploded' })).toContain('tool body exploded');
    expect(hostEventBanner({ type: 'taken_over', from: 'tui' })).toContain('tui');
    expect(hostEventBanner({ type: 'reloaded' })).toBeTruthy();
    expect(hostEventBanner({ type: 'agent_end' })).toBeNull();
  });

  it('narrows a resume patch and a snapshot apart', () => {
    const resume = SessionSnapshotSchema.parse({
      session_id: 's1',
      reset: false,
      seq: 3,
      replay: [{ session: 's1', seq: 3, event: { type: 'agent_end' } }],
    });
    expect(resume.reset).toBe(false);
    expect(resume.entries).toBeUndefined();
    const full = SessionSnapshotSchema.parse({ session_id: 's1', reset: true, seq: 0, entries: [] });
    expect(full.entries).toEqual([]);
  });
});
