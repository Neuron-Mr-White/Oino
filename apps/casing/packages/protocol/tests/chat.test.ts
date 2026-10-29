import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import { ChatCloseDataSchema, ChatConfigDataSchema, ChatNewDataSchema, ControlSchema, PROTO_VERSION } from '../src/index.js';
import { atLeast } from './semver.js';

type Fixture = { name: string; schema: string; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(readFileSync(new URL('../../../protocol/fixtures/chat.json', import.meta.url), 'utf8'));
const schemas: Record<string, z.ZodTypeAny> = {
  Control: ControlSchema,
  ChatNewData: ChatNewDataSchema,
  ChatCloseData: ChatCloseDataSchema,
  ChatConfigData: ChatConfigDataSchema,
};

describe('chat_new / chat_close / chat_config contract fixtures (UNI-260)', () => {
  it('proto is 1.12.0+', () => expect(atLeast(PROTO_VERSION, '1.12.0')).toBe(true));
  for (const fixture of fixtures) {
    it(fixture.name, () => {
      const schema = schemas[fixture.schema];
      expect(schema, fixture.schema).toBeDefined();
      expect(schema!.safeParse(fixture.value).success).toBe(fixture.ok);
    });
  }
});
