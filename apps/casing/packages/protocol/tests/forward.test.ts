import { readFileSync } from 'node:fs';
import { describe, it, expect } from 'vitest';
import { z } from 'zod';
import { ControlSchema, PortsListDataSchema, PROTO_VERSION } from '../src/index.js';
import { atLeast } from './semver.js';

type Schema = 'Control' | 'PortsListData';
type Fixture = { name: string; schema: Schema; value: unknown; ok: boolean };
const fixtures: Fixture[] = JSON.parse(readFileSync(new URL('../../../protocol/fixtures/forward.json', import.meta.url), 'utf8'));
const schemas: Record<Schema, z.ZodTypeAny> = { Control: ControlSchema, PortsListData: PortsListDataSchema };

describe('ports_list / tcp_forward contract fixtures (UNI-213)', () => {
  it('proto is 1.6.0+ (forwarding)', () => expect(atLeast(PROTO_VERSION, '1.6.0')).toBe(true));
  for (const fixture of fixtures) {
    it(fixture.name, () => {
      const result = schemas[fixture.schema].safeParse(fixture.value);
      expect(result.success).toBe(fixture.ok);
    });
  }
});
