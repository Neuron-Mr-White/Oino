// protocol/fixtures/session_bulk.json against the zod schemas ([ux-sessions]).
import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { z } from 'zod';
import {
  HubEventSchema,
  SessionsArchiveRequestSchema,
  SessionsBatchDataSchema,
  SessionsQueryDataSchema,
  SessionsQueryRequestSchema,
  SessionsRestoreRequestSchema,
} from '../src/index.js';

type Fixture = { name: string; schema: string; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(readFileSync(new URL('../../../protocol/fixtures/session_bulk.json', import.meta.url), 'utf8'));
const schemas: Record<string, z.ZodType> = {
  Request: z.union([SessionsArchiveRequestSchema, SessionsRestoreRequestSchema, SessionsQueryRequestSchema]),
  SessionsBatchData: SessionsBatchDataSchema,
  SessionsQueryData: SessionsQueryDataSchema,
  HubEvent: HubEventSchema,
};

describe('session_bulk fixtures', () => {
  for (const f of fixtures) {
    it(f.name, () => {
      expect(schemas[f.schema]!.safeParse(f.value).success).toBe(f.ok);
    });
  }
});
